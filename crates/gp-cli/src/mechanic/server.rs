//! `gp mechanic start`: the page's server, on 127.0.0.1 only. It serves the
//! page (`mechanic/` in the repo), the player module (`gp_play.wasm`, built
//! by scripts/web/build-play.sh into target/play-web, or `$GP_PLAY_WASM`)
//! and the ROM the game's roms.toml names, found by hash in $GP_ROM_DIR.
//! For an arcade game (its package's platform), the player is
//! gp_arcade_play.wasm (`scripts/web/build-play.sh --arcade`, or
//! `$GP_ARCADE_PLAY_WASM`), served under the same name, and the ROM is the
//! romset's zip; its clips' logs are arcade input logs.
//! Every write comes from the page itself: a POST from another origin is
//! refused.
//!
//! | Route | What |
//! |---|---|
//! | `GET /`, `/app.js`, `/nesplayer.js`, `/style.css` | The page |
//! | `GET /gp_play.wasm`, `/rom` | The player and the ROM |
//! | `GET /api/session` | The open session's state ([`Session::state`]) |
//! | `GET /api/working` | The mechanic the agent works on and its newest clip, for the idle screen |
//! | `GET /api/clips/N.log` | A clip as its full log from power-on |
//! | `GET /api/clips/N.draws` | The original's random draws over a clip as the page saw them while recording it (`POST /api/clips`'s `draws`, kept as `clips/NNN.draws`); 404 when none were kept |
//! | `GET /api/live` | The agent's model playing live: always empty here |
//! | `POST /api/paths` | Keeps the play so far as a path |
//! | `POST /api/clips` | Saves a clip: a path, a start and its own inputs, or a full log and a start; with a path, `draws` (a draws script) keeps the original's random draws the page saw over it beside it |
//! | `POST /api/talk` | The person's turn |
//! | `POST /api/pick` | Opens another mechanic of the quest log; nothing goes to the agent |
//! | `POST /api/handoff` | Hands a quest log row to the agent: its "Let's do" turn |
//! | `GET /api/clips/N/review.json`, `/api/clips/N/review/NAME.png` | A clip on the model beside the original ([`super::review`]) |
//! | `GET /api/clips/N.rebuild.json` | The same review as REVIEW REBUILD reads it: the pictures' addresses with the entities, what differs and the draws |
//! | `GET /api/clips/N.measure.json` | A clip's measurements, when `gp mechanic contacts` wrote them |
//! | `POST /api/clips/stage` | Whether a clip about to be handed off spans a stage |
//! | `GET /api/sealed`, `/api/sealed/ID.log` | The sealed runs, for the person alone; nothing is written |
//! | `GET /api/rebuild/scores` | The scoring status, `$GP_HOME/work/refine/GAME/status.json`, sealed rows left out |
//! | `GET /api/rebuild/status`, `/api/rebuild/profiles`, `POST /api/rebuild/build`, `/api/rebuild/start`, `GET /rebuild/*` | The rebuild played live ([`super::live`]) |
//! | `GET /api/rebuild/status?ask=ID`, `POST /api/rebuild/build` and `/api/rebuild/start` with `ask`, `GET /rebuild-ask/ID/*` | A gate card's look: the model as the card's patch would leave it ([`super::ask_look`]) |

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow, bail};
use gp_evidence::Paths;
use serde_json::{Value, json};

use super::http::{self, Request, Response};
use super::paths as clip_paths;
use super::session::{Session, current, queue_file, read_queue, set_current, write_queue};
use crate::Outcome;
use crate::original::Platform;

/// What `gp mechanic start` was asked.
#[derive(Clone, Copy)]
pub(crate) struct Start<'a> {
    pub game: &'a str,
    pub mechanic: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub port: u16,
    pub games: Option<&'a Path>,
    /// The game's built module, for the review of clips on the model.
    pub module: Option<&'a Path>,
    /// Whether to restart into a newly published `$GP_HOME/bin/gp`.
    pub follow: bool,
}

/// The largest request body: a play from power-on, about 11 bytes a frame,
/// for hours.
const MAX_BODY: usize = 16 << 20;

/// The repo holding the page (`mechanic/index.html`): the working folder or
/// one above it, else the checkout gp was built from.
pub(crate) fn repo_root() -> Result<PathBuf> {
    crate::repo::find_repo(|d| d.join("mechanic/index.html").is_file()).ok_or_else(|| {
        anyhow!("no mechanic/ page found: run gp mechanic start inside the gameprinter repo")
    })
}

/// The player module: `$GP_PLAY_WASM`, else the repo's
/// target/play-web/gp_play.wasm; for an arcade game,
/// `$GP_ARCADE_PLAY_WASM`, else target/play-web/gp_arcade_play.wasm.
fn wasm_path(repo: &Path, platform: Platform) -> PathBuf {
    let (var, file) = match platform {
        Platform::Nes => ("GP_PLAY_WASM", "gp_play.wasm"),
        Platform::Arcade => ("GP_ARCADE_PLAY_WASM", "gp_arcade_play.wasm"),
    };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map_or_else(|| repo.join("target/play-web").join(file), PathBuf::from)
}

/// A game's ROM sets: its roms.toml's sources, the package's scored set
/// (package.toml's `role = "scored"`, the one the clips are of) first, then
/// the rest in roms.toml's order; and the file.
fn sets_of(
    games: Option<&Path>,
    game: &str,
) -> Result<(
    PathBuf,
    Vec<gp_evidence::registry::KnownSource>,
    Vec<String>,
)> {
    let games = crate::roms_cmd::games_dir(games)
        .context("no games checkout: pass --games or set GP_GAMES_DIR")?;
    let file = games.join(game).join("roms.toml");
    let text = fs::read_to_string(&file).with_context(|| {
        format!(
            "no {}: is {game} a game in {}?",
            file.display(),
            games.display()
        )
    })?;
    let reg = gp_evidence::registry::Registry::from_toml(&text, &file)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let scored: Vec<String> = crate::package::Package::load(&games.join(game))
        .map(|p| {
            p.entries
                .into_iter()
                .filter(|e| e.role == "scored")
                .map(|e| e.sha256)
                .collect()
        })
        .unwrap_or_default();
    let mut sources = reg.sources;
    sources.sort_by_key(|s| (!scored.contains(&s.sha256), s.id != game));
    Ok((file, sources, scored))
}

/// Where the person's choice of a game's ROM set is kept: its id, one line.
fn choice_file(paths: &Paths, game: &str) -> Result<PathBuf> {
    Ok(super::session::game_dir(paths, game)?.join("rom-set"))
}

/// The ROM set chosen for a game, if one was.
fn chosen(paths: &Paths, game: &str) -> Option<String> {
    let t = fs::read_to_string(choice_file(paths, game).ok()?).ok()?;
    Some(t.trim().to_owned()).filter(|t| !t.is_empty())
}

/// Chooses the ROM set a game's page plays, by its id in roms.toml
/// or by a file of it (a zip or a .nes): kept for the game, so the page,
/// gp serve and gp mechanic start all play it until another is chosen. The
/// set's id.
pub(crate) fn choose_rom(
    paths: &Paths,
    games: Option<&Path>,
    game: &str,
    asked: &str,
) -> Result<String> {
    let (file, sources, _) = sets_of(games, game)?;
    let p = Path::new(asked);
    let id = if p.is_file() {
        let rom = crate::roms::Rom::read(p)?;
        sources
            .iter()
            .find(|s| s.sha256 == rom.sha256)
            .map(|s| s.id.clone())
            .with_context(|| {
                format!(
                    "{asked} (sha256 {}) isn't a set {} names",
                    rom.sha256,
                    file.display()
                )
            })?
    } else {
        sources
            .iter()
            .find(|s| s.id == asked || s.system.as_deref() == Some(asked))
            .map(|s| s.id.clone())
            .with_context(|| {
                format!(
                    "{} names no set {asked}: it has {}",
                    file.display(),
                    sources
                        .iter()
                        .map(|s| s.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?
    };
    let f = choice_file(paths, game)?;
    if let Some(d) = f.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(&f, format!("{id}\n"))?;
    Ok(id)
}

/// The game's ROM: the set chosen for it (see [`choose_rom`]) when it is in
/// $GP_ROM_DIR, else the scored set, else the rest in roms.toml's order,
/// found by hash. The ROM, its set's id, and every set with whether it is
/// here (for the page's picker).
pub(super) fn game_rom(
    paths: &Paths,
    games: Option<&Path>,
    game: &str,
) -> Result<(crate::roms::Rom, String, Vec<Value>)> {
    let (file, mut sources, scored) = sets_of(games, game)?;
    let picked = chosen(paths, game);
    if let Some(id) = &picked {
        sources.sort_by_key(|s| &s.id != id);
    }
    let shas: Vec<String> = sources.iter().map(|s| s.sha256.clone()).collect();
    let mut found = crate::roms::find(&shas, &[], &paths.roms)?;
    let list: Vec<Value> = sources
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "title": s.title,
                "revision": s.revision,
                "system": s.system,
                "sha256": s.sha256,
                "scored": scored.contains(&s.sha256),
                "here": found.contains_key(&s.sha256),
            })
        })
        .collect();
    for s in &sources {
        if let Some(r) = found.remove(&s.sha256) {
            return Ok((r, s.id.clone(), list));
        }
    }
    bail!(
        "no ROM in {} that {} names: add yours with gp roms add",
        paths.roms.display(),
        file.display()
    )
}

/// The ROM a clip plays on, whatever set the page's picker holds: the set
/// it was recorded on (`set`, its log's `system` or a set's id in
/// roms.toml), else the scored set (the one the corpus's clips are of),
/// else the rest in roms.toml's order. The ROM and its set's id; a set
/// that isn't in the ROM folder is refused in plain words.
pub(crate) fn clip_rom(
    paths: &Paths,
    games: Option<&Path>,
    game: &str,
    set: Option<&str>,
) -> Result<(crate::roms::Rom, String)> {
    let (file, sources, _) = sets_of(games, game)?;
    if let Some(want) = set {
        let s = sources
            .iter()
            .find(|s| s.id == want || s.system.as_deref() == Some(want))
            .with_context(|| {
                format!(
                    "the clip was recorded on {want}, a set {} doesn't name",
                    file.display()
                )
            })?;
        let mut found = crate::roms::find(std::slice::from_ref(&s.sha256), &[], &paths.roms)?;
        let r = found.remove(&s.sha256).with_context(|| {
            format!(
                "the clip was recorded on {}, which isn't in {}: add it with gp roms add",
                s.id,
                paths.roms.display()
            )
        })?;
        return Ok((r, s.id.clone()));
    }
    let shas: Vec<String> = sources.iter().map(|s| s.sha256.clone()).collect();
    let mut found = crate::roms::find(&shas, &[], &paths.roms)?;
    for s in &sources {
        if let Some(r) = found.remove(&s.sha256) {
            return Ok((r, s.id.clone()));
        }
    }
    bail!(
        "no ROM in {} that {} names: add yours with gp roms add",
        paths.roms.display(),
        file.display()
    )
}

/// GET /rom?set=SET: the bytes of the game's set `want` (its id or MAME
/// system), the one the page plays when it is that set; a set the game
/// doesn't have, or one not in the ROM folder, said in plain words.
fn set_rom(s: &Server, want: &str) -> Result<Response> {
    let Some(row) = s
        .sets
        .iter()
        .find(|r| r["id"] == json!(want) || r["system"] == json!(want))
    else {
        return Ok(Response::error(
            404,
            &format!(
                "the clip was recorded on {want}, a set {} doesn't have",
                s.game
            ),
        ));
    };
    let sha = row["sha256"].as_str().unwrap_or("");
    if sha == s.rom_sha {
        return Ok(Response::bytes(
            "application/octet-stream",
            s.rom.as_ref().clone(),
        ));
    }
    let id = row["id"].as_str().unwrap_or(want);
    let mut found = crate::roms::find(&[sha.to_owned()], &[], &s.paths.roms)?;
    match found.remove(sha) {
        Some(r) => Ok(Response::bytes("application/octet-stream", r.bytes)),
        None => Ok(Response::error(
            409,
            &format!(
                "the clip was recorded on {id}, which isn't in {}: add it with gp roms add",
                s.paths.roms.display()
            ),
        )),
    }
}

/// The set an input log was recorded on: an arcade log's `system`.
pub(crate) fn log_set(log: &str) -> Option<String> {
    gp_evidence::InputLog::parse(log)
        .ok()?
        .arcade
        .map(|a| a.system)
        .filter(|s| !s.is_empty())
}

/// The model's pictures for a ROM set: the scored set's in the usual place
/// (corpus_score::assets_of, $GP_HOME/work/assets/SOURCE); another set's
/// lifted by the same asset map, chip by chip and index by index, into
/// $GP_HOME/work/assets/SOURCE@SET, so the model draws with that set's art.
fn set_assets(
    paths: &Paths,
    game_dir: &Path,
    rom: &[u8],
    set: &str,
    scored: bool,
) -> Result<PathBuf> {
    if scored {
        return crate::corpus_score::assets_of(paths, game_dir, game_dir, None, rom);
    }
    let file = game_dir.join(gp_model::MODEL_FILE);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let model = gp_model::Model::parse(&text).with_context(|| file.display().to_string())?;
    let source = model
        .model
        .source
        .context("the model names no source to lift its pictures for")?;
    let out = paths
        .home
        .join("work")
        .join("assets")
        .join(format!("{source}@{set}"));
    crate::corpus_score::lift_into(game_dir, rom, &out)?;
    Ok(out)
}

/// The mechanic to open: the one asked for, else the one open last, else
/// the first quest log row not done.
fn mechanic_to_open(paths: &Paths, game: &str, asked: Option<&str>) -> Result<String> {
    if let Some(m) = asked {
        return Ok(m.to_owned());
    }
    if let Ok(m) = current(paths, game)
        && Session::exists(paths, game, &m)?
    {
        return Ok(m);
    }
    read_queue(&queue_file(paths, game)?)
        .iter()
        .find(|r| r["status"] != json!("done"))
        .and_then(|r| r["mechanic"].as_str().map(str::to_owned))
        .with_context(|| {
            format!("{game} has no quest log row to open: name a mechanic, as gp mechanic start {game} jump")
        })
}

/// The server's listening socket, on this machine only.
fn listen(port: u16) -> Result<TcpListener> {
    // A build the server restarted into may find its port held for a
    // moment, so it tries again for a few seconds.
    let tries = if std::env::var_os(RESTARTED).is_some() {
        50
    } else {
        1
    };
    let mut last = None;
    for _ in 0..tries {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => return Ok(l),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(anyhow::Error::new(last.expect("one try at least")))
        .with_context(|| format!("port {port} is taken: give another with --port"))
}

/// Set for the build a server restarts into.
const RESTARTED: &str = "GP_MECHANIC_RESTARTED";

/// What `bin/gp` resolves to and when that file was written: a change
/// means a new build was published.
pub(crate) fn published_id(link: &Path) -> Option<(std::path::PathBuf, std::time::SystemTime)> {
    let real = fs::canonicalize(link).ok()?;
    let when = fs::metadata(&real).ok()?.modified().ok()?;
    Some((real, when))
}

/// Whether `bin --version` exits 0 within ten seconds.
pub(crate) fn runs(bin: &Path) -> bool {
    let Ok(mut child) = std::process::Command::new(bin)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return s.success(),
            Ok(None) if started.elapsed() < std::time::Duration::from_secs(10) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Replaces this process with `bin`, serving the same port and the
/// mechanic open now, with the options it was started with. The listening
/// socket closes with the exec and the new build binds the port again, a
/// fraction of a second later. Returns only when the exec fails.
fn restart(srv: &Server, bin: &Path, a: &Start) -> anyhow::Error {
    let mechanic = srv
        .session
        .lock()
        .expect("session lock")
        .as_ref()
        .and_then(|s| s.dir.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    eprintln!(
        "gp mechanic: a new build is published; restarting into {}",
        bin.display()
    );
    let mut c = std::process::Command::new(bin);
    c.args([
        "mechanic",
        "start",
        &srv.game,
        &mechanic,
        "--own-server",
        "--port",
    ])
    .arg(srv.port.to_string());
    for (flag, v) in [("--games", a.games), ("--module", a.module)] {
        if let Some(v) = v {
            c.arg(flag).arg(v);
        }
    }
    c.env(RESTARTED, "1");
    exec(c)
}

#[cfg(unix)]
pub(crate) fn exec(mut c: std::process::Command) -> anyhow::Error {
    use std::os::unix::process::CommandExt as _;
    anyhow::Error::new(c.exec())
}

/// Windows can't replace a running process with another, and the new
/// build couldn't bind the port this one holds, so --follow says so and
/// stays on this build.
#[cfg(not(unix))]
pub(crate) fn exec(c: std::process::Command) -> anyhow::Error {
    let _ = c;
    anyhow::anyhow!(
        "this system can't restart a server in place; stop gp mechanic and start it again"
    )
}

/// What the page says beside an arcade game's model on review.
const ARCADE_ART: &str = "For now an arcade game's model is drawn by tile kind, as blocks of color where the original shows the game's art.";

/// The page's server for a game, ready to answer at `port` (its own, or
/// `gp serve`'s, which mounts it at /mechanic/GAME/), with the mechanic
/// `a` asks for open: its session made when new.
pub(crate) fn prepare(paths: &Paths, a: &Start, port: u16) -> Result<(Arc<Server>, String)> {
    let repo = repo_root()?;
    let (rom, set, sets) = game_rom(paths, a.games, a.game)?;
    let platform = rom.platform;
    let scored = sets
        .iter()
        .any(|s| s["id"] == json!(set) && s["scored"] == json!(true));
    let mut server = Server::new(paths.clone(), a.game, &repo, rom.bytes, &rom.sha256, port);
    server.platform = platform;
    server.wasm = wasm_path(&repo, platform);
    server.set = set.clone();
    server.sets = sets;
    if let Some(games) = crate::roms_cmd::games_dir(a.games) {
        let game_dir = games.join(a.game);
        let mut rb = super::review::Rebuild::new(paths, &game_dir, a.game, a.module);
        rb.rom = Some((server.rom.clone(), server.rom_sha.clone()));
        // A game with no model has no pictures to lift: the page says so
        // where the model would play.
        rb.assets = set_assets(paths, &game_dir, &server.rom, &set, scored).ok();
        server.rebuild = Some(rb);
    }
    let server = Arc::new(server);
    // The playable rebuild kept as the model stands: looked at every half
    // minute, built again in the background when it is older.
    {
        let s = server.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(30));
                if let Some(rb) = &s.rebuild {
                    super::live::refresh(&s.paths, &s.repo, &s.game, rb, &s.rom);
                    // The open gate cards' looks, so LOOK AT IT plays
                    // their change without a wait.
                    super::ask_look::refresh(&s.paths, &s.repo, &s.game, rb, &s.rom);
                }
            }
        });
    }
    let mechanic = server.open_mechanic(a.mechanic, a.prompt)?;
    Ok((server, mechanic))
}

/// A request to a prepared server, answered as its own server answers it.
pub(crate) fn answer(s: &Server, req: &Request) -> Response {
    handle(s, req).unwrap_or_else(|e| Response::error(500, &format!("{e:#}")))
}

/// What to tell the person's agent about a session.
pub(crate) fn agent_words(game: &str, mechanic: &str) -> String {
    format!(
        "Give your agent this: \"We're working on the {mechanic} of {game}. Run `gp mechanic show {game} {mechanic}` to see the session, answer with `gp mechanic say {game} {mechanic} \\\"...\\\"`, and keep the loop going: ask me for the clip you need next.\""
    )
}

/// `gp mechanic start --own-server`: the page on a server of its own.
pub(crate) fn start(paths: &Paths, a: &Start) -> Result<Outcome> {
    let listener = listen(a.port)?;
    let port = listener.local_addr()?.port();
    let (server, mechanic) = prepare(paths, a, port)?;
    let game = a.game;
    println!("{game} / {mechanic}: http://127.0.0.1:{port}/");
    println!("Session: {}", server.open_session()?.dir.display());
    println!("{}", agent_words(game, &mechanic));
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut server = server;
    // Accepted here only, so with none in flight no request is running:
    // the server restarts into a new published build then, never during a
    // request or a rebuild's build.
    let busy = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let link = paths.home.join("bin").join("gp");
    let mut seen = published_id(&link);
    let mut looked = std::time::Instant::now();
    listener.set_nonblocking(true)?;
    loop {
        match listener.accept() {
            Ok((conn, _)) => {
                // Accepted sockets take the listener's non-blocking flag on
                // macOS.
                let _ = conn.set_nonblocking(false);
                // Another ROM set chosen in the page: the server is made
                // again for it before the page's next request.
                if server.wants_reload() {
                    match prepare(
                        paths,
                        &Start {
                            mechanic: None,
                            prompt: None,
                            ..*a
                        },
                        port,
                    ) {
                        Ok((s, _)) => server = s,
                        Err(e) => {
                            eprintln!("gp mechanic start: the set chosen couldn't start: {e:#}")
                        }
                    }
                }
                let s = server.clone();
                let b = busy.clone();
                b.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                std::thread::spawn(move || {
                    let _ = serve_one(&s, &conn);
                    drop(conn);
                    b.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if a.follow && looked.elapsed() >= std::time::Duration::from_secs(2) {
                    looked = std::time::Instant::now();
                    let now = published_id(&link);
                    if let Some((bin, _)) = now.as_ref().filter(|_| now != seen)
                        && busy.load(std::sync::atomic::Ordering::SeqCst) == 0
                        && !super::live::building()
                    {
                        if runs(bin) {
                            let e = restart(&server, bin, a);
                            eprintln!(
                                "gp mechanic: couldn't restart into {}: {e:#}",
                                bin.display()
                            );
                        } else {
                            eprintln!(
                                "gp mechanic: {} doesn't run `--version`; staying on this build",
                                bin.display()
                            );
                        }
                        seen = now;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => {}
        }
    }
}

impl Server {
    /// Opens a mechanic in the page: the one asked for, else the one open
    /// last or the quest log's first not done. A session there already, or
    /// a quest log row (whose new session gets its first turn, as a pick
    /// from the page does), else a new mechanic with the person's words.
    pub(crate) fn open_mechanic(
        &self,
        asked: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<String> {
        let (paths, game) = (&self.paths, self.game.as_str());
        let mechanic = mechanic_to_open(paths, game, asked)?;
        let in_log = read_queue(&queue_file(paths, game)?)
            .iter()
            .any(|r| r["mechanic"] == json!(mechanic));
        let session = if Session::exists(paths, game, &mechanic)? || (in_log && prompt.is_none()) {
            self.session_named(&mechanic)?
        } else {
            let s = Arc::new(Session::create(
                paths,
                game,
                &mechanic,
                prompt,
                &self.rom_sha,
            )?);
            self.open
                .lock()
                .expect("open lock")
                .insert(mechanic.clone(), s.clone());
            s
        };
        *self.session.lock().expect("session lock") = Some(session);
        set_current(paths, game, &mechanic)?;
        Ok(mechanic)
    }

    /// The session open now, for `gp serve`.
    pub(crate) fn session_dir(&self) -> Result<PathBuf> {
        Ok(self.open_session()?.dir.clone())
    }
}

pub(crate) struct Server {
    /// The mechanic open in the page; picking another from the quest log
    /// swaps it.
    session: Mutex<Option<Arc<Session>>>,
    /// Every session this server has opened, by mechanic, so two requests
    /// for one mechanic share its lock.
    open: Mutex<std::collections::HashMap<String, Arc<Session>>>,
    paths: Paths,
    game: String,
    rom_sha: String,
    /// The page's files.
    page: PathBuf,
    /// The repo they are in: the preview's build script and page.
    repo: PathBuf,
    wasm: PathBuf,
    rom: Arc<Vec<u8>>,
    /// The machine the ROM is for, which picks the player module.
    platform: Platform,
    port: u16,
    /// What the review of clips on the model needs; none without a games
    /// checkout.
    rebuild: Option<super::review::Rebuild>,
    /// The ROM set played (its id in roms.toml), and every set of the game
    /// with whether it is in $GP_ROM_DIR.
    set: String,
    sets: Vec<Value>,
    /// Another set was chosen: the server is made again for it.
    reload: std::sync::atomic::AtomicBool,
}

impl Server {
    fn new(paths: Paths, game: &str, repo: &Path, rom: Vec<u8>, rom_sha: &str, port: u16) -> Self {
        Self {
            session: Mutex::new(None),
            open: Mutex::default(),
            paths,
            game: game.to_owned(),
            rom_sha: rom_sha.to_owned(),
            page: repo.join("mechanic"),
            repo: repo.to_path_buf(),
            wasm: wasm_path(repo, Platform::Nes),
            rom: Arc::new(rom),
            platform: Platform::Nes,
            port,
            rebuild: None,
            set: String::new(),
            sets: Vec::new(),
            reload: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Whether another ROM set was chosen since the server was made: then
    /// whoever holds it makes it again (gp serve's mount, or the server of
    /// its own), and the page loads again.
    pub(crate) fn wants_reload(&self) -> bool {
        self.reload.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn queue_file(&self) -> Result<PathBuf> {
        queue_file(&self.paths, &self.game)
    }

    /// The session open now.
    fn open_session(&self) -> Result<Arc<Session>> {
        self.session
            .lock()
            .expect("session lock")
            .clone()
            .context("no mechanic open")
    }

    /// A mechanic's session, opened once for the server's life (made, with
    /// no turn, when it is new and in the quest log: opening a row hands
    /// nothing to the agent; [`Self::hand_off`] does).
    fn session_named(&self, m: &str) -> Result<Arc<Session>> {
        let mut open = self.open.lock().expect("open lock");
        if let Some(s) = open.get(m) {
            return Ok(s.clone());
        }
        let session = if Session::exists(&self.paths, &self.game, m)? {
            Session::open(&self.paths, &self.game, m)?
        } else {
            let q = read_queue(&self.queue_file()?);
            let Some(row) = q.iter().find(|r| r["mechanic"] == json!(m)) else {
                bail!("{m} is not in the quest log");
            };
            let note = row["note"].as_str().unwrap_or("").to_owned();
            Session::create(&self.paths, &self.game, m, Some(&note), &self.rom_sha)?
        };
        let session = Arc::new(session);
        open.insert(m.to_owned(), session.clone());
        Ok(session)
    }

    /// The person hands a row to the agent (the open row's HAND TO THE
    /// AGENT): its "Let's do" turn, which makes the row the agent's. A row
    /// whose last turn is the person's is the agent's already; nothing is
    /// added. Whether a turn was added.
    fn hand_off(&self, m: &str) -> Result<bool> {
        let q = read_queue(&self.queue_file()?);
        let Some(row) = q.iter().find(|r| r["mechanic"] == json!(m)) else {
            bail!("{m} is not in the quest log");
        };
        let session = self.session_named(m)?;
        if session
            .turns()?
            .last()
            .is_some_and(|t| t["by"] == json!("player"))
        {
            return Ok(false);
        }
        let note = row["note"].as_str().unwrap_or("");
        session
            .turn(json!({"by": "player", "text": format!("Let's do {m}: {note}"), "clips": []}))?;
        Ok(true)
    }

    /// The sealed runs' store, `$GP_HOME/sealed/GAME/sealed-run/clips/`:
    /// outside every mechanic's session, so no worker reading the sessions
    /// sees them. Kept as a session for its numbering and its lock.
    fn sealed(&self) -> Result<Arc<Session>> {
        let mut open = self.open.lock().expect("open lock");
        if let Some(s) = open.get(SEALED_KEY) {
            return Ok(s.clone());
        }
        let dir = self
            .paths
            .home
            .join("sealed")
            .join(&self.game)
            .join(SEALED_ROW);
        let s = Arc::new(Session::store(dir)?);
        open.insert(SEALED_KEY.to_owned(), s.clone());
        Ok(s)
    }

    /// What the rebuild played live needs, or the words for why there is
    /// none. An arcade game's model plays in the same interpreter preview,
    /// keyed as a cabinet, its board art drawn by the page's gp_board.wasm
    /// (issue #3).
    fn live(&self) -> std::result::Result<&super::review::Rebuild, Response> {
        self.rebuild()
    }

    /// What a clip's review on the model needs, or the words for why there
    /// is none.
    fn rebuild(&self) -> std::result::Result<&super::review::Rebuild, Response> {
        self.rebuild.as_ref().ok_or_else(|| {
            Response::error(
                404,
                "no games checkout, so no model to play: start the page with --games",
            )
        })
    }

    /// A review on the model with, on an arcade game whose model names
    /// none of its board's art, the sentence the page shows beside it
    /// (`art`): the model's side is drawn by tile kind, so no one reads
    /// its blocks as the model's art gone wrong.
    fn art_said(&self, mut v: Value) -> Value {
        if self.platform == Platform::Arcade && !self.board_art() {
            v["art"] = json!(ARCADE_ART);
        }
        v
    }

    /// Whether the game's model names its board's own art (a level's
    /// `present.board` or an entity's `board_sprite`,
    /// docs/formats/board-art.md), so its side isn't drawn by tile kind.
    fn board_art(&self) -> bool {
        self.rebuild
            .as_ref()
            .and_then(|r| std::fs::read_to_string(r.game.join("model.toml")).ok())
            .and_then(|t| gp_model::Model::parse(&t).ok())
            .is_some_and(|m| gp_look::board::names_board(&m))
    }

    /// A point-and-tell request (the person paused, boxed part of the
    /// screen and said what is wrong there) as the person's turn in the
    /// open mechanic's talk, with its `point`: where the agent reads the
    /// talk already (`gp mechanic show`), and answers with `gp mechanic
    /// say`. The turn's number and the mechanic.
    pub(crate) fn point_turn(&self, text: &str, point: Value) -> Result<(usize, String)> {
        let s = self.open_session()?;
        let n = s.turn(json!({"by": "player", "text": text, "clips": [], "point": point}))?;
        Ok((n, s.mechanic().to_owned()))
    }

    /// A tell about a clip (serve/tell.rs) echoed as the person's turn in
    /// the open mechanic's talk, with a `tell` field naming the clip and
    /// the tell's number, so the page's conversation column shows it. The
    /// turn's number and the mechanic.
    pub(crate) fn tell_turn(&self, text: &str, tell: Value) -> Result<(usize, String)> {
        let s = self.open_session()?;
        let n = s.turn(json!({"by": "player", "text": text, "clips": [], "tell": tell}))?;
        Ok((n, s.mechanic().to_owned()))
    }

    /// The session a request names with `mechanic` (the row the page had
    /// open when the person pressed A), else the one open now.
    fn target(&self, m: Option<&str>) -> Result<Arc<Session>> {
        match m {
            Some(m) => self.session_named(m),
            None => self.open_session(),
        }
    }
}

/// A quest-log row's name from the person's words: their first four words
/// of letters and digits, lower case, joined by dashes (`item` if none).
fn row_name(text: &str) -> String {
    let words: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(4)
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        "item".to_owned()
    } else {
        words.join("-")
    }
}

fn serve_one(s: &Server, conn: &TcpStream) -> std::io::Result<()> {
    let req = match http::read_request_up_to(conn, MAX_BODY) {
        Ok(r) => r,
        Err(http::ReadError::Bad(status, why)) => {
            return http::write_response(conn, &Response::error(status, why), false);
        }
        Err(http::ReadError::Gone) => return Ok(()),
    };
    // A watch session's event stream (crate::watch_serve) is written as it goes.
    if req.own_host(s.port)
        && let Some(done) = crate::watch_serve::stream(&req, conn)
    {
        return done;
    }
    let res = handle(s, &req).unwrap_or_else(|e| Response::error(500, &format!("{e:#}")));
    http::write_response(conn, &res, req.method == "HEAD")
}

/// The quest-log row sealed runs are reported under, and the key of their
/// store among the open sessions (no mechanic's name has a colon).
const SEALED_ROW: &str = "sealed-run";
const SEALED_KEY: &str = "sealed:";

/// A clip number from a route's middle (`/api/clips/12.log`).
fn clip_number(p: &str, prefix: &str, suffix: &str) -> Option<u64> {
    p.strip_prefix(prefix)?.strip_suffix(suffix)?.parse().ok()
}

fn handle(s: &Server, req: &Request) -> Result<Response> {
    let page = |name: &str, kind: &'static str| -> Result<Response> {
        Ok(Response::bytes(kind, fs::read(s.page.join(name))?))
    };
    if !req.own_host(s.port) {
        return Ok(Response::error(
            403,
            "this page answers only at 127.0.0.1 or localhost on its own port",
        ));
    }
    let post = req.method == "POST";
    if post && !req.same_origin(s.port) {
        return Ok(Response::error(403, "only this server's own page writes"));
    }
    // Watch sessions' routes (crate::watch_serve, docs/watch.md).
    if let Some(r) = crate::watch_serve::handle(req) {
        return r;
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET" | "HEAD", "/") => page("index.html", "text/html; charset=utf-8"),
        ("GET" | "HEAD", "/app.js") => page("app.js", "text/javascript; charset=utf-8"),
        ("GET" | "HEAD", "/rebuild-card.js") => {
            page("rebuild-card.js", "text/javascript; charset=utf-8")
        }
        ("GET" | "HEAD", "/nesplayer.js") => page("nesplayer.js", "text/javascript; charset=utf-8"),
        ("GET" | "HEAD", "/style.css") => page("style.css", "text/css; charset=utf-8"),
        // The refinement view: the rule `gp rule` found and the last
        // `gp model refine` run on this game.
        ("GET" | "HEAD", "/refine") => page("refine.html", "text/html; charset=utf-8"),
        ("GET" | "HEAD", "/refine.js") => page("refine.js", "text/javascript; charset=utf-8"),
        ("GET" | "HEAD", "/refine.css") => page("refine.css", "text/css; charset=utf-8"),
        ("GET" | "HEAD", "/api/refine") => {
            Ok(Response::json(&crate::refine::last_run(&s.paths, &s.game)))
        }
        (_, p) if p.starts_with("/api/rule") => Ok(crate::rule_refine::route(
            &s.paths.home.join("mechanics").join(&s.game),
            req,
        )?
        .unwrap_or_else(|| Response::error(404, "not found"))),
        // The screen (mechanic/screen.html): the original and the model on
        // one screen, which the agent drives (gp_mcp::screen).
        ("GET" | "HEAD", "/screen.html") => page("screen.html", "text/html; charset=utf-8"),
        ("GET" | "HEAD", "/screen.js") => page("screen.js", "text/javascript; charset=utf-8"),
        ("GET" | "HEAD", "/screen.css") => page("screen.css", "text/css; charset=utf-8"),
        ("GET" | "HEAD", "/api/screen") => screen_state(s),
        ("POST", "/api/screen") => {
            let v: Value = serde_json::from_slice(&req.body)?;
            match gp_mcp::screen::report(&s.paths.home, &s.game, &v) {
                Ok(v) => Ok(Response::json(&v)),
                Err(e) => Ok(Response::error(400, &e)),
            }
        }
        ("GET" | "HEAD", "/api/screen/clip") => screen_clip(s, req),
        ("GET" | "HEAD", "/api/screen/work-clip") => screen_work_clip(s, req),
        ("GET" | "HEAD", "/shell.js") => page("shell.js", "text/javascript; charset=utf-8"),
        // The status line: where the port stands and the next step.
        ("GET" | "HEAD", "/next.js") => page("next.js", "text/javascript; charset=utf-8"),
        // A picture the agent proposes for the person's theme (issue #4).
        ("GET" | "HEAD", "/picture-card.js") => {
            page("picture-card.js", "text/javascript; charset=utf-8")
        }
        ("GET" | "HEAD", "/picture-card.css") => {
            page("picture-card.css", "text/css; charset=utf-8")
        }
        // A sound change the agent proposes for the person's copy, in the
        // picture card's look.
        ("GET" | "HEAD", "/sound-card.js") => {
            page("sound-card.js", "text/javascript; charset=utf-8")
        }
        // A sprite edit the agent proposes for the person's copy, the same.
        ("GET" | "HEAD", "/sprite-card.js") => {
            page("sprite-card.js", "text/javascript; charset=utf-8")
        }
        // The old inspector's views on the page (mechanic/details/).
        ("GET" | "HEAD", p) if part_in(p, "/details/").is_some() => {
            let (name, kind) = part_in(p, "/details/").unwrap_or_default();
            match fs::read(s.page.join("details").join(name)) {
                Ok(b) => Ok(Response::bytes(kind, b)),
                Err(_) => Ok(Response::error(404, "not here")),
            }
        }
        // REFINE's parts (the background work's views), when they're there.
        ("GET" | "HEAD", p) if part_name(p).is_some() => {
            let (name, kind) = part_name(p).unwrap_or_default();
            match fs::read(s.page.join("refine").join(name)) {
                Ok(b) => Ok(Response::bytes(kind, b)),
                Err(_) => Ok(Response::error(404, "not here")),
            }
        }
        ("GET" | "HEAD", "/api/themes") => Ok(Response::json(&themes(s))),
        // THEMES's own routes and the pane's files (crate::theme_serve).
        (_, p)
            if (p.starts_with("/api/themes/") || p.starts_with("/themes/"))
                && s.rebuild.is_some() =>
        {
            let r = s
                .rebuild
                .as_ref()
                .map(|r| r.game.clone())
                .unwrap_or_default();
            let at = crate::theme_serve::At {
                paths: &s.paths,
                game: &s.game,
                dir: &r,
                page: &s.page,
            };
            crate::theme_serve::route(&at, req)
                .unwrap_or_else(|| Ok(Response::error(404, "not found")))
        }
        // The game's ROM sets: the one played and every one with whether
        // it is here; a POST {set} chooses another.
        ("GET" | "HEAD", "/api/rom") => Ok(Response::json(
            &json!({"set": s.set, "sha256": s.rom_sha, "sets": s.sets}),
        )),
        // An error the page showed the person: the loop files it as a
        // tools row of its own (crate::trouble).
        ("POST", "/api/trouble") => {
            let v: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let text = v["text"].as_str().unwrap_or("").trim();
            if text.is_empty() {
                return Ok(Response::error(400, "say what the page showed (text)"));
            }
            let place = v["where"].as_str().unwrap_or("");
            Ok(Response::json(&crate::trouble::from_page(
                &s.paths, &s.game, text, place,
            )?))
        }
        ("POST", "/api/rom") => {
            let v: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let want = v["set"].as_str().unwrap_or("");
            let Some(row) = s.sets.iter().find(|r| r["id"] == json!(want)) else {
                return Ok(Response::error(
                    404,
                    &format!("{} has no set {want}", s.game),
                ));
            };
            if row["here"] != json!(true) {
                return Ok(Response::error(
                    409,
                    &format!(
                        "{want} isn't in {}: add it with gp roms add",
                        s.paths.roms.display()
                    ),
                ));
            }
            fs::create_dir_all(super::session::game_dir(&s.paths, &s.game)?)?;
            fs::write(choice_file(&s.paths, &s.game)?, format!("{want}\n"))?;
            if want != s.set {
                s.reload.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(Response::json(
                &json!({"set": want, "reload": want != s.set}),
            ))
        }
        ("GET" | "HEAD", "/gp_play.wasm") => {
            if !s.wasm.is_file() {
                return Ok(Response::error(
                    404,
                    &format!(
                        "the player isn't built: run scripts/web/build-play.sh{} (no {})",
                        if s.platform == Platform::Arcade {
                            " --arcade"
                        } else {
                            ""
                        },
                        s.wasm.display()
                    ),
                ));
            }
            Ok(Response::bytes("application/wasm", fs::read(&s.wasm)?))
        }
        // The ROM the page plays; ?set=SET (a set's id or MAME system), the
        // set a clip was recorded on, which the page replays it on whatever
        // set its picker holds.
        ("GET" | "HEAD", "/rom") => match req.param("set").filter(|v| !v.is_empty()) {
            None => Ok(Response::bytes(
                "application/octet-stream",
                s.rom.as_ref().clone(),
            )),
            Some(want) => set_rom(s, want),
        },
        // The arcade sets gp knows (the games' roms.toml chip rows), which
        // the page hands its board player (gp_sets_load) before the zip.
        ("GET" | "HEAD", "/roms.toml") => Ok(Response::bytes(
            "text/plain; charset=utf-8",
            gp_arcade::romset::known_roms_toml().into_bytes(),
        )),
        ("GET" | "HEAD", "/api/session") => {
            let mut st = s.open_session()?.state()?;
            // While no agent connected to gp mcp this week, the page says
            // how to connect one (gp_mcp::clients).
            if let Some(o) = st.as_object_mut() {
                let now = gp_mcp::asks::now_ms();
                let line = gp_mcp::clients::no_agent(&s.paths.home, now);
                o.insert("no_agent".into(), json!(line));
            }
            Ok(Response::json(&st))
        }
        ("GET" | "HEAD", "/api/working") => working(s, req.param("mechanic")),
        // Every quest-log row's clips and talk, for the screen page's list
        // under the screens, where the person comments on a row or a clip:
        // a row with no session has none (none is made here).
        ("GET" | "HEAD", "/api/screen/clips") => {
            let mut out = Vec::new();
            for r in read_queue(&s.queue_file()?) {
                let Some(m) = r["mechanic"].as_str() else {
                    continue;
                };
                if !Session::exists(&s.paths, &s.game, m).unwrap_or(false) {
                    continue;
                }
                let session = Session::open(&s.paths, &s.game, m)?;
                let talk = session.turns()?;
                // The row's status line and whose turn it is, so YOUR TURN
                // can show a question its agent asked with an ANSWER box.
                let busy = super::session::busy_of(session.busy_file().as_ref(), &talk);
                let waiting = super::session::waiting_for(&talk);
                out.push(
                    json!({"mechanic": m, "clips": session.clips()?, "talk": talk,
                    "busy": busy, "waiting_for": waiting}),
                );
            }
            Ok(Response::json(&json!({"mechanics": out})))
        }
        ("GET" | "HEAD", p) if clip_number(p, "/api/clips/", ".log").is_some() => {
            let n = clip_number(p, "/api/clips/", ".log").unwrap_or(0);
            let session = s.target(req.param("mechanic"))?;
            if !session.dir.join(format!("clips/{n:03}.toml")).is_file() {
                return Ok(Response::error(404, "no such clip"));
            }
            let (log, _) = clip_paths::full_log(&s.paths.home, &session.dir, n)?;
            Ok(Response::bytes(
                "text/plain; charset=utf-8",
                log.into_bytes(),
            ))
        }
        // The original's random draws over a clip as the page saw them
        // when it was recorded (docs/formats/draws.md's text form).
        ("GET" | "HEAD", p) if clip_number(p, "/api/clips/", ".draws").is_some() => {
            let n = clip_number(p, "/api/clips/", ".draws").unwrap_or(0);
            let session = s.target(req.param("mechanic"))?;
            match fs::read(session.dir.join(format!("clips/{n:03}.draws"))) {
                Ok(b) => Ok(Response::bytes("text/plain; charset=utf-8", b)),
                Err(_) => Ok(Response::error(404, "no draws kept with this clip")),
            }
        }
        ("GET" | "HEAD", "/api/live") => {
            // The demo's lines (`gp mechanic demo`) from `after` on: the page
            // plays them as they come.
            let after: usize = req.param("after").and_then(|a| a.parse().ok()).unwrap_or(0);
            let f = s.open_session()?.dir.join("live.ndjson");
            let lines: Vec<Value> = fs::read_to_string(f)
                .unwrap_or_default()
                .lines()
                .skip(after)
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            Ok(Response::json(&json!({"lines": lines})))
        }
        // A clip played on the model beside the original ([`super::review`]):
        // made on first ask, which takes as long as scoring the clip.
        ("GET" | "HEAD", p) if clip_number(p, "/api/clips/", "/review.json").is_some() => {
            let n = clip_number(p, "/api/clips/", "/review.json").unwrap_or(0);
            let session = s.target(req.param("mechanic"))?;
            if !session.dir.join(format!("clips/{n:03}.toml")).is_file() {
                return Ok(Response::error(404, "no such clip"));
            }
            let Some(r) = &s.rebuild else {
                return Ok(Response::error(
                    404,
                    "no games checkout, so no model to review on: start the page with --games",
                ));
            };
            match super::review::review(&s.paths, r, &session.dir, n) {
                Ok(v) => Ok(Response::json(&s.art_said(v))),
                Err(e) => Ok(Response::error(409, &format!("{e:#}"))),
            }
        }
        ("GET" | "HEAD", p) if p.starts_with("/api/clips/") && p.contains("/review/") => {
            let rest = &p["/api/clips/".len()..];
            let Some((n, name)) = rest
                .split_once("/review/")
                .and_then(|(n, name)| Some((n.parse::<u64>().ok()?, name)))
            else {
                return Ok(Response::error(404, "no such picture"));
            };
            let session = s.target(req.param("mechanic"))?;
            match super::review::picture(&session.dir, n, name) {
                Some(b) => Ok(Response::bytes("image/png", b)),
                None => Ok(Response::error(404, "no such picture")),
            }
        }
        // What the agents are doing in every topic of the game, for the
        // watch panel ([`super::watch::game_view`]); `?mechanic=` is the one
        // open in the page.
        ("GET" | "HEAD", "/api/watch") => {
            let open = s.target(req.param("mechanic"))?;
            let game = super::session::game_dir(&s.paths, &s.game)?;
            let name = open.dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
            Ok(Response::json(&super::watch::game_view(&game, name)))
        }
        ("GET" | "HEAD", p) if p.starts_with("/api/watch/files/") => {
            let name = &p["/api/watch/files/".len()..];
            let found = match req.param("topic") {
                Some(t) => {
                    super::watch::topic_file(&super::session::game_dir(&s.paths, &s.game)?, t, name)
                }
                None => super::watch::file(&s.target(req.param("mechanic"))?.dir, name),
            };
            match found {
                Some((kind, bytes)) => Ok(Response::bytes(kind, bytes)),
                None => Ok(Response::error(404, "no such pinned file")),
            }
        }
        // A stored path's log, for a request's PLAY FROM HERE (gp mechanic
        // triage): the page replays it to the request's frame.
        ("GET" | "HEAD", p) if p.starts_with("/api/paths/") => {
            let sha = &p["/api/paths/".len()..];
            match clip_paths::read_path(&s.paths.home, sha) {
                Ok(log) => Ok(Response::bytes(
                    "text/plain; charset=utf-8",
                    log.to_text().into_bytes(),
                )),
                Err(e) => Ok(Response::error(404, &format!("{e:#}"))),
            }
        }
        ("POST", "/api/paths") => {
            // A real-input log from power-on, kept once by its hash: the whole
            // log, or `base` (a path already stored) and the frames after it.
            let v: Value = serde_json::from_slice(&req.body)?;
            let home = &s.paths.home;
            let (path, frames) = match (v["base"].as_str(), v["tail"].as_str(), v["log"].as_str()) {
                (Some(b), Some(t), _) => {
                    let tail = gp_evidence::input_log::InputLog::parse(t)
                        .map_err(|e| anyhow::anyhow!("the tail: {e}"))?;
                    clip_paths::extend_path(home, b, &tail.frames)?
                }
                (_, _, Some(l)) => clip_paths::put_path(home, l)?,
                _ => return Ok(Response::error(400, "send {log} or {base, tail}")),
            };
            // `warm`: a frame a clip is about to start from (the page sends
            // it when a recording starts). The replay to it runs now, in the
            // background, and leaves the start the hand-off's state hash
            // reads.
            if let Some(w) = v["warm"].as_u64() {
                let (home, rom, sha) = (home.clone(), s.rom.clone(), path.clone());
                std::thread::spawn(move || {
                    let _ = clip_paths::original_at(&home, &rom, &sha, w as usize);
                });
            }
            Ok(Response::json(&json!({"path": path, "frames": frames})))
        }
        ("POST", "/api/clips") => post_clip(s, &serde_json::from_slice(&req.body)?),
        ("POST", "/api/talk") => {
            let v: Value = serde_json::from_slice(&req.body)?;
            let clips: Vec<u64> = v["clips"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_u64).collect())
                .unwrap_or_default();
            let mut t =
                json!({"by": "player", "text": v["text"].as_str().unwrap_or(""), "clips": clips});
            if let Some(c) = v["choice"].as_u64() {
                t["choice"] = json!(c);
            }
            // The model's pick the person saw, and whether they agreed:
            // the record of how often it is right.
            if let Some(l) = v["laya"].as_str() {
                t["laya"] = json!(l);
                t["agreed"] = json!(v["text"].as_str() == Some(l));
            }
            let text = v["text"].as_str().unwrap_or("").to_owned();
            let session = s.target(v["mechanic"].as_str())?;
            // A plain yes on a row asking to confirm a look is CONFIRM: the
            // runner locks it, no worker (crate::approved::confirm).
            let qf = s.queue_file()?;
            let me = session.mechanic().to_owned();
            let q = read_queue(&qf);
            let row = q.iter().find(|r| r["mechanic"] == json!(me));
            let yes = crate::approved::says_yes(&text);
            let confirms = yes && row.is_some_and(crate::approved::confirmable);
            let (n, again) = session.turn_once(t)?;
            if confirms {
                let looks = crate::approved::saved_looks(None, &s.game);
                crate::approved::confirm(&qf, &s.game, &looks, &me, &text, true)?;
            }
            Ok(Response::json(
                &json!({"turn": n, "again": again, "confirmed": confirms}),
            ))
        }
        // CONFIRM on a row asking the person to confirm a look: their yes,
        // which gp serve's runner locks with no worker.
        ("POST", "/api/confirm") => {
            let v: Value = serde_json::from_slice(&req.body)?;
            let m = v["mechanic"].as_str().context("mechanic")?;
            let looks = crate::approved::saved_looks(None, &s.game);
            match crate::approved::confirm(&s.queue_file()?, &s.game, &looks, m, "CONFIRM", false) {
                Ok(r) => Ok(Response::json(&json!({"mechanic": m, "row": r}))),
                Err(e) => Ok(Response::error(409, &format!("{e:#}"))),
            }
        }
        // A new quest-log row from the person's words (the screen page's NEW
        // ITEM): named from its first words, made unique, status "next" and
        // the words as its note, so `next` drives it from now on.
        ("POST", "/api/screen/row") => {
            let v: Value = serde_json::from_slice(&req.body)?;
            let text = v["text"].as_str().unwrap_or("").trim().to_owned();
            if text.is_empty() {
                return Ok(Response::error(400, "say what the item is"));
            }
            let f = s.queue_file()?;
            let mut q = read_queue(&f);
            let base = row_name(&text);
            let mut name = base.clone();
            let mut k = 2;
            while q.iter().any(|r| r["mechanic"] == json!(name)) {
                name = format!("{base}-{k}");
                k += 1;
            }
            q.push(json!({"mechanic": name, "status": "next", "note": text, "by": "person"}));
            write_queue(&f, &q)?;
            Ok(Response::json(&json!({"mechanic": name})))
        }
        ("POST", "/api/handoff") => {
            let v: Value = serde_json::from_slice(&req.body)?;
            let m = v["mechanic"].as_str().context("mechanic")?;
            if !read_queue(&s.queue_file()?)
                .iter()
                .any(|r| r["mechanic"] == json!(m))
            {
                return Ok(Response::error(404, "not in the quest log"));
            }
            let added = s.hand_off(m)?;
            Ok(Response::json(&json!({"mechanic": m, "again": !added})))
        }
        ("POST", "/api/pick") => {
            // The person picked a mechanic from the quest log: open its
            // session (made if new) in the page. Nothing goes to the agent
            // and no status changes; HAND TO THE AGENT (/api/handoff) does
            // that.
            let v: Value = serde_json::from_slice(&req.body)?;
            let m = v["mechanic"].as_str().context("mechanic")?;
            if !read_queue(&s.queue_file()?)
                .iter()
                .any(|r| r["mechanic"] == json!(m))
            {
                return Ok(Response::error(404, "not in the quest log"));
            }
            let session = s.session_named(m)?;
            set_current(&s.paths, &s.game, m)?;
            *s.session.lock().expect("session lock") = Some(session);
            Ok(Response::json(&json!({"mechanic": m})))
        }
        ("GET" | "HEAD", p) if clip_number(p, "/api/clips/", ".rebuild.json").is_some() => {
            let n = clip_number(p, "/api/clips/", ".rebuild.json").unwrap_or(0);
            let session = s.target(req.param("mechanic"))?;
            if !session.dir.join(format!("clips/{n:03}.toml")).is_file() {
                return Ok(Response::error(404, "no such clip"));
            }
            let rb = match s.rebuild() {
                Ok(rb) => rb,
                Err(r) => return Ok(r),
            };
            match super::review::review(&s.paths, rb, &session.dir, n) {
                Ok(v) => Ok(Response::json(&s.art_said(super::review::as_rebuild(
                    &v,
                    n,
                    session.mechanic(),
                )))),
                Err(e) => Ok(Response::error(409, &format!("{e:#}"))),
            }
        }
        ("GET" | "HEAD", p) if clip_number(p, "/api/clips/", ".measure.json").is_some() => {
            let n = clip_number(p, "/api/clips/", ".measure.json").unwrap_or(0);
            let f = s
                .target(req.param("mechanic"))?
                .dir
                .join(format!("clips/{n:03}.measure.json"));
            if !f.is_file() {
                return Ok(Response::error(
                    404,
                    "not measured: gp mechanic contacts writes a clip's measurements, and it isn't in gameprinter yet",
                ));
            }
            Ok(Response::bytes("application/json", fs::read(f)?))
        }
        ("POST", "/api/clips/stage") => {
            // Whether a clip about to be handed off spans a stage: the page
            // then offers to seal it, as the default.
            let v: Value = serde_json::from_slice(&req.body)?;
            let inputs =
                gp_evidence::input_log::InputLog::parse(v["inputs"].as_str().context("inputs")?)
                    .map_err(|e| anyhow::anyhow!("the clip's inputs: {e}"))?;
            Ok(Response::json(&stage_span(
                s,
                v["path"].as_str().context("path")?,
                v["start"].as_u64().context("start")? as usize,
                &inputs,
            )?))
        }
        // The sealed runs, private to the person: read only when the page's
        // sealed view asks, and nothing about them is written or kept.
        ("GET" | "HEAD", "/api/sealed") => {
            let runs: Vec<Value> = sealed_runs(&s.paths.home, &s.game)
                .into_iter()
                .map(|(id, toml)| {
                    let t: toml::Table = fs::read_to_string(&toml)
                        .ok()
                        .and_then(|t| toml::from_str(&t).ok())
                        .unwrap_or_default();
                    json!({"id": id, "start": t.get("start").and_then(|v| v.as_integer()),
                           "frames": t.get("frames").and_then(|v| v.as_integer()),
                           "note": t.get("note").and_then(|v| v.as_str())})
                })
                .collect();
            Ok(Response::json(&json!({"runs": runs})))
        }
        ("GET" | "HEAD", p) if p.starts_with("/api/sealed/") => {
            let rest = &p["/api/sealed/".len()..];
            let (id, what) = rest.split_once('.').unwrap_or((rest, ""));
            let Some((_, toml)) = sealed_runs(&s.paths.home, &s.game)
                .into_iter()
                .find(|(i, _)| i == id)
            else {
                return Ok(Response::error(404, "no such sealed run"));
            };
            match what {
                "log" => Ok(Response::bytes(
                    "text/plain; charset=utf-8",
                    sealed_log(&s.paths.home, &toml)?.into_bytes(),
                )),
                "rebuild.json" => Ok(Response::error(
                    409,
                    "a sealed run on the model waits for gp sealed (being ported in gameprinter-wt-port1): the review keeps its pictures beside the clip, and nothing of a sealed run may be kept",
                )),
                _ => Ok(Response::error(404, "a sealed run has .log")),
            }
        }
        ("GET" | "HEAD", "/api/rebuild/scores") => scores(s),
        // The rebuild played live: the interpreter preview of the game's
        // model, built on request into $GP_HOME/work/mechanic-preview/GAME/
        // and driven by the page.
        // A gate card's look (`ask`): the model as the card's patch would
        // leave it, built and played beside main's (super::ask_look).
        ("GET" | "HEAD", "/api/rebuild/status") => match (s.live(), req.param("ask")) {
            (Ok(_), Some(id)) => {
                let v = super::ask_look::status(&s.paths, &s.game, id);
                Ok(Response::json(&v))
            }
            (Ok(rb), None) => {
                let v = super::live::status(&s.paths, &s.repo, &s.game, rb);
                Ok(Response::json(&v))
            }
            (Err(r), _) => Ok(r),
        },
        ("GET" | "HEAD", "/api/rebuild/profiles") => match s.live() {
            Ok(rb) => Ok(Response::json(&super::live::profiles(
                &s.paths, &s.game, rb,
            )?)),
            Err(r) => Ok(r),
        },
        ("POST", "/api/rebuild/build") => match s.live() {
            Ok(rb) => {
                let v: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                if let Some(id) = v["ask"].as_str() {
                    return Ok(build_look(s, rb, id));
                }
                if let Err(e) = super::live::build(&s.paths, &s.repo, &s.game, rb, &s.rom) {
                    return Ok(Response::error(409, &format!("{e:#}")));
                }
                Ok(Response::json(&super::live::status(
                    &s.paths, &s.repo, &s.game, rb,
                )))
            }
            Err(r) => Ok(r),
        },
        ("POST", "/api/rebuild/start") => match s.live() {
            Ok(main) => {
                let v: Value = serde_json::from_slice(&req.body)?;
                let ask = match v["ask"].as_str() {
                    Some(id) => match super::ask_look::rebuild(&s.paths, &s.game, main, id) {
                        Ok(r) => Some(r),
                        Err(e) => return Ok(Response::error(409, &format!("{e:#}"))),
                    },
                    None => None,
                };
                let rb = ask.as_ref().unwrap_or(main);
                match super::live::start(
                    &s.paths,
                    rb,
                    &s.rom,
                    v["path"].as_str().context("path")?,
                    v["frame"].as_u64().context("frame")?,
                ) {
                    Ok(v) => Ok(Response::json(&v)),
                    Err(e) => Ok(Response::error(409, &format!("{e:#}"))),
                }
            }
            Err(r) => Ok(r),
        },
        // An arcade model sounds through the board's own sound CPU, which
        // the rebuild's frame runs from the player (the same build as
        // /gp_play.wasm), the sets gp knows and the session's ROM, which
        // the page hands it (`__gpRom`, mechanic/app.js).
        ("GET" | "HEAD", "/rebuild/gp_arcade_play.wasm") if s.platform == Platform::Arcade => {
            match fs::read(&s.wasm) {
                Ok(b) => Ok(Response::bytes("application/wasm", b)),
                Err(_) => Ok(Response::error(
                    404,
                    "the arcade player isn't built: run scripts/web/build-play.sh --arcade",
                )),
            }
        }
        ("GET" | "HEAD", "/rebuild/lift/sources.toml") => Ok(Response::bytes(
            "text/plain; charset=utf-8",
            gp_arcade::romset::known_roms_toml().into_bytes(),
        )),
        // A gate card's look's preview: the arcade player and the sets gp
        // knows as main's, the rest from the look's build.
        ("GET" | "HEAD", p) if p.starts_with("/rebuild-ask/") => {
            let rest = &p["/rebuild-ask/".len()..];
            let name = rest.split_once('/').map_or("", |(_, n)| n);
            if name == "gp_arcade_play.wasm" && s.platform == Platform::Arcade {
                return Ok(match fs::read(&s.wasm) {
                    Ok(b) => Response::bytes("application/wasm", b),
                    Err(_) => Response::error(404, "the arcade player isn't built"),
                });
            }
            if name == "lift/sources.toml" {
                return Ok(Response::bytes(
                    "text/plain; charset=utf-8",
                    gp_arcade::romset::known_roms_toml().into_bytes(),
                ));
            }
            super::ask_look::file(&s.paths, &s.repo, &s.game, rest)
        }
        ("GET" | "HEAD", p) if p.starts_with("/rebuild/") => {
            super::live::file(&s.paths, &s.repo, &s.game, &p["/rebuild/".len()..])
        }
        _ => Ok(Response::error(404, "not found")),
    }
}

/// `POST /api/rebuild/build` with `ask`: a gate card's look built
/// ([`super::ask_look`]), and its status.
fn build_look(s: &Server, rb: &super::review::Rebuild, id: &str) -> Response {
    match super::ask_look::build(&s.paths, &s.repo, &s.game, rb, &s.rom, id) {
        Ok(()) => Response::json(&super::ask_look::status(&s.paths, &s.game, id)),
        Err(e) => Response::error(409, &format!("{e:#}")),
    }
}

/// The scoring pipeline's status for REVIEW REBUILD's clip picker and its
/// refinement panel, `$GP_HOME/work/refine/GAME/status.json`, with its time
/// so the page redraws only when it changes. A sealed row is left out.
fn scores(s: &Server) -> Result<Response> {
    let f = s
        .paths
        .home
        .join("work/refine")
        .join(&s.game)
        .join("status.json");
    let Ok(meta) = fs::metadata(&f) else {
        return Ok(Response::json(&json!({"status": null})));
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64);
    let mut v: Value = serde_json::from_str(&fs::read_to_string(&f)?)
        .with_context(|| format!("reading {}", f.display()))?;
    let shown = |m: &str| !m.starts_with("sealed");
    for k in ["rows", "clips"] {
        if let Some(a) = v[k].as_array_mut() {
            a.retain(|r| shown(r["mechanic"].as_str().unwrap_or("sealed")));
        }
    }
    Ok(Response::json(&json!({"status": v, "mtime": mtime})))
}

/// Whether a clip spans a stage: the stage the game's observe.toml
/// `[level]` reads (its RAM byte and the names of its values) moves from
/// one to another during it, as a clear does. On an arcade game the byte
/// is the main CPU's, read from gp-arcade at each frame of the clip.
fn stage_span(
    s: &Server,
    path: &str,
    start: usize,
    inputs: &gp_evidence::InputLog,
) -> Result<Value> {
    let Some(games) = s.rebuild.as_ref().map(|r| r.game.clone()) else {
        return Ok(json!({"spans": false, "why": "no games checkout"}));
    };
    let Some((addr, names)) = level_key(&games.join("observe.toml")) else {
        return Ok(json!({"spans": false, "why": "the game's observe.toml names no stage"}));
    };
    if s.platform == Platform::Arcade {
        let mut o = clip_paths::original_at(&s.paths.home, &s.rom, path, start)?;
        return Ok(spans_of(&stages_on(&mut o, addr, &names, inputs)?));
    }
    let mut nes = clip_paths::nes_at(&s.paths.home, &s.rom, path, start)?;
    // Only RAM is read: no pictures.
    nes.set_video(false);
    let mut seen: Vec<(String, usize)> = Vec::new();
    let mut note = |nes: &gp_nes::Nes, at: usize| {
        let v = nes.wram()[usize::from(addr) & 0x7ff];
        if let Some(l) = names.get(&v)
            && seen.last().is_none_or(|(p, _)| p != l)
        {
            seen.push((l.clone(), at));
        }
    };
    note(&nes, 0);
    for (i, f) in inputs.frames.iter().enumerate() {
        nes.step(*f)?;
        note(&nes, i + 1);
    }
    Ok(spans_of(&seen))
}

/// The stages a clip passes through, each with the frame it starts on.
fn spans_of(seen: &[(String, usize)]) -> Value {
    json!({
        "spans": seen.len() >= 2,
        "stages": seen.iter().map(|(l, f)| json!({"stage": l, "from": f})).collect::<Vec<_>>(),
    })
}

/// The stages the original passes through over `inputs`, read from its
/// `ram[addr]` before the first frame and after each: on either platform,
/// through the runtime's own selector.
fn stages_on(
    o: &mut crate::original::OriginalRuntime,
    addr: u16,
    names: &std::collections::HashMap<u8, String>,
    inputs: &gp_evidence::InputLog,
) -> Result<Vec<(String, usize)>> {
    use gp_runtime::Runtime as _;
    let sel = format!("ram[{addr:#06x}]");
    let mut seen: Vec<(String, usize)> = Vec::new();
    let mut note = |o: &crate::original::OriginalRuntime, at: usize| -> Result<()> {
        let v = o
            .query(&sel)
            .map_err(anyhow::Error::msg)?
            .first()
            .and_then(|e| e.value.as_int())
            .with_context(|| format!("{sel} reads no number"))?;
        if let Some(l) = names.get(&(v as u8))
            && seen.last().is_none_or(|(p, _)| p != l)
        {
            seen.push((l.clone(), at));
        }
        Ok(())
    };
    note(o, 0)?;
    for (i, f) in crate::run::frames(inputs).into_iter().enumerate() {
        o.step(&[f]).map_err(anyhow::Error::msg)?;
        note(o, i + 1)?;
    }
    Ok(seen)
}

/// An observe.toml's `[level]`: the RAM byte and each value's name.
fn level_key(file: &Path) -> Option<(u16, std::collections::HashMap<u8, String>)> {
    let t: toml::Table = toml::from_str(&fs::read_to_string(file).ok()?).ok()?;
    let level = t.get("level")?.as_table()?;
    let addr = u16::try_from(level.get("ram")?.as_array()?.first()?.as_integer()?).ok()?;
    let mut names = std::collections::HashMap::new();
    for pair in level.get("names")?.as_array()? {
        let pair = pair.as_array()?;
        let (v, name) = (pair.first()?.as_str()?, pair.get(1)?.as_str()?);
        let v = u8::from_str_radix(v.trim_start_matches("0x"), 16).ok()?;
        names.insert(v, name.to_owned());
    }
    Some((addr, names))
}

/// A file of REFINE's parts: `/refine/NAME.js` or `.css`, a plain name.
fn part_name(p: &str) -> Option<(&str, &'static str)> {
    part_in(p, "/refine/")
}

/// A file of a folder of the page's parts: `PREFIX` and a plain name,
/// `.js` or `.css`.
fn part_in<'a>(p: &'a str, prefix: &str) -> Option<(&'a str, &'static str)> {
    let name = p.strip_prefix(prefix)?;
    let ok = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        && !name.starts_with('.');
    let kind = if name.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else {
        return None;
    };
    ok.then_some((name, kind))
}

/// THEMES's view: the content packages the person's registry
/// (`$GP_HOME/registry/catalog.toml`) offers for this game, and the files
/// of the game's theme kit (games/GAME/themes/kit).
fn themes(s: &Server) -> Value {
    let catalog = s.paths.home.join("registry").join("catalog.toml");
    let suffix = format!("/{}", s.game);
    let list: Vec<Value> = fs::read_to_string(&catalog)
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("package").and_then(|p| p.as_array()).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            p.get("kind").and_then(|k| k.as_str()) == Some("content")
                && p.get("for_games")
                    .and_then(|g| g.as_array())
                    .is_some_and(|g| {
                        g.iter()
                            .any(|n| n.as_str().is_some_and(|n| n.ends_with(&suffix)))
                    })
        })
        .map(|p| {
            json!({
                "name": p.get("name").and_then(|v| v.as_str()),
                "title": p.get("title").and_then(|v| v.as_str()),
                "latest": p.get("latest").and_then(|v| v.as_str()),
            })
        })
        .collect();
    let mut kit: Vec<String> = s
        .rebuild
        .as_ref()
        .and_then(|r| fs::read_dir(r.game.join("themes").join("kit")).ok())
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    kit.sort();
    json!({
        "registry": catalog.is_file().then(|| catalog.display().to_string()),
        "themes": list,
        "kit": kit,
    })
}

/// The sealed runs (`$GP_HOME/sealed/GAME/`: its top folder's clips and the
/// page's `sealed-run/clips/`), by an id the page names them with. Read
/// only when the person opens the sealed view; nothing is written.
fn sealed_runs(home: &Path, game: &str) -> Vec<(String, PathBuf)> {
    let top = home.join("sealed").join(game);
    let mut out = Vec::new();
    for (dir, prefix) in [
        (top.clone(), ""),
        (top.join(SEALED_ROW).join("clips"), "sealed-run-"),
    ] {
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "toml") && p.with_extension("log").is_file() {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let id = format!("{prefix}{stem}");
                if super::name_ok(&id) {
                    out.push((id, p));
                }
            }
        }
    }
    out.sort();
    out
}

/// A sealed run's whole input log from power-on, for the page's original.
fn sealed_log(home: &Path, toml: &Path) -> Result<String> {
    let t: toml::Table = toml::from_str(&fs::read_to_string(toml)?)?;
    let own = fs::read_to_string(toml.with_extension("log"))?;
    let Some(sha) = t.get("path").and_then(|v| v.as_str()) else {
        return Ok(own);
    };
    let start = t
        .get("start")
        .and_then(|v| v.as_integer())
        .unwrap_or(0)
        .max(0) as usize;
    let mut log = clip_paths::read_path(home, sha)?;
    log.frames.truncate(start);
    let clip = gp_evidence::InputLog::parse(&own)
        .map_err(|e| anyhow::anyhow!("{}: {e}", toml.display()))?;
    log.frames.extend(clip.frames);
    Ok(log.to_text())
}

/// The mechanic the agent is on now (or `?mechanic=NAME`) and its newest
/// clip: the page loops it while the person is idle.
fn working(s: &Server, asked: Option<&str>) -> Result<Response> {
    let q = read_queue(&s.queue_file()?);
    let Some(m) = q
        .iter()
        .find(|r| match asked {
            Some(a) => r["mechanic"] == json!(a),
            None => r["status"] == json!("agent working"),
        })
        .and_then(|r| r["mechanic"].as_str())
    else {
        return Ok(Response::json(&json!({"mechanic": null})));
    };
    let Ok(w) = Session::open(&s.paths, &s.game, m) else {
        return Ok(Response::json(&json!({"mechanic": null})));
    };
    let st = w.state()?;
    let Some(c) = st["clips"].as_array().and_then(|c| c.last()).cloned() else {
        return Ok(Response::json(&json!({"mechanic": m, "busy": st["busy"]})));
    };
    let n = c["n"].as_u64().unwrap_or(0);
    let (log, _) = clip_paths::full_log(&s.paths.home, &w.dir, n)?;
    Ok(Response::json(
        &json!({"mechanic": m, "busy": st["busy"], "clip": c, "log": log}),
    ))
}

/// `POST /api/clips`: a clip filed under the mechanic the page names,
/// whatever is open now. Either a path, a start on it and the clip's own
/// inputs, or a full log from power-on and a start.
/// `GET /api/screen`: the agent's last ask for the screen, what the page
/// reported last, and each of the person's themes for the game with the
/// time its newest file was written, so the page shows a theme's change
/// by itself.
fn screen_state(s: &Server) -> Result<Response> {
    let home = &s.paths.home;
    let ask = gp_mcp::screen::ask_of(home, &s.game).map_err(anyhow::Error::msg)?;
    let shown = gp_mcp::screen::shown_of(home, &s.game).map_err(anyhow::Error::msg)?;
    let work = gp_mcp::screen::work_of(home, &s.game).map_err(anyhow::Error::msg)?;
    let themes: serde_json::Map<String, Value> = super::live::theme_times(&s.paths, &s.game)
        .into_iter()
        .map(|(id, t)| (id, json!(t)))
        .collect();
    Ok(Response::json(
        &json!({"ask": ask, "shown": shown, "work": work, "themes": themes}),
    ))
}

/// `GET /api/screen/work-clip?id=ID`: a corpus clip of the game, as the
/// screen plays it while it follows a refine: its play from power-on, the
/// frame it starts at and its length, with the mapping's draw sites
/// (`draw_sites`) for the screen to record the original's random draws at
/// and hand to the model, as for a session clip (`draws` is null: a corpus
/// clip keeps none of its own).
fn screen_work_clip(s: &Server, req: &Request) -> Result<Response> {
    let Some(id) = req.param("id") else {
        return Ok(Response::error(400, "id is the corpus clip's"));
    };
    let Some(rb) = &s.rebuild else {
        return Ok(Response::error(404, "no games checkout: no corpus"));
    };
    let Ok(c) = crate::clip_score::corpus_clip(&rb.game, id) else {
        return Ok(Response::error(404, "no such corpus clip"));
    };
    let (log, start) = match c.log() {
        Ok(l) => l,
        Err(e) => return Ok(Response::error(404, &format!("{e:#}"))),
    };
    let frames = log.frames.len().saturating_sub(start);
    // The mapping's draw sites, so the screen records the original's
    // random draws from the replay and hands them to the model beside it.
    let sites = super::live::draw_sites_of(rb);
    Ok(Response::json(&json!({
        "id": id,
        "name": c.recipe.name,
        "start": start,
        "frames": frames,
        "log": log.to_text(),
        "draws": Value::Null,
        "draw_sites": sites,
    })))
}

/// `GET /api/screen/clip?n=N&mechanic=M`: where a clip starts on its play
/// and how long it is, for the screen to play it, with the original's
/// random draws kept with it (`draws`, or null) and the mapping's draw
/// sites (`draw_sites`, as `/api/rebuild/start` gives them).
fn screen_clip(s: &Server, req: &Request) -> Result<Response> {
    let Some(n) = req.param("n").and_then(|n| n.parse::<u64>().ok()) else {
        return Ok(Response::error(400, "n is the clip's number"));
    };
    let session = s.target(req.param("mechanic"))?;
    let f = session.dir.join(format!("clips/{n:03}.toml"));
    let Ok(text) = fs::read_to_string(&f) else {
        return Ok(Response::error(404, "no such clip"));
    };
    let t: toml::Table = toml::from_str(&text)?;
    // The original's random draws over the clip, kept when it was recorded
    // on the page, and the sites to record them at again when not: the
    // model beside the clip takes them, as it does live.
    let draws = fs::read_to_string(session.dir.join(format!("clips/{n:03}.draws"))).ok();
    let sites = s.rebuild().map_or(Value::Null, super::live::draw_sites_of);
    Ok(Response::json(&json!({
        "n": n,
        "mechanic": session.mechanic(),
        "start": t.get("start").and_then(|v| v.as_integer()),
        "frames": t.get("frames").and_then(|v| v.as_integer()),
        "note": t.get("note").and_then(|v| v.as_str()),
        "draws": draws,
        "draw_sites": sites,
    })))
}

fn post_clip(s: &Server, v: &Value) -> Result<Response> {
    // A sealed run goes to the sealed store, whatever row is open.
    let sealed = v["sealed"] == json!(true);
    let session = if sealed {
        s.sealed()?
    } else {
        s.target(v["mechanic"].as_str())?
    };
    let note = v["note"].as_str().unwrap_or("");
    let Some(path) = v["path"].as_str() else {
        let n = session.add_clip(
            v["log"].as_str().context("log")?,
            v["start"].as_u64().context("start")?,
            note,
            "player",
        )?;
        return Ok(Response::json(
            &json!({"n": n, "again": false, "mechanic": session.mechanic()}),
        ));
    };
    let inputs = gp_evidence::input_log::InputLog::parse(v["inputs"].as_str().context("inputs")?)
        .map_err(|e| anyhow::anyhow!("the clip's inputs: {e}"))?;
    let start = v["start"].as_u64().context("start")? as usize;
    if inputs.frames.is_empty() {
        bail!("the clip has no frames");
    }
    let have = clip_paths::read_path(&s.paths.home, path)?.frames.len();
    if start > have {
        bail!("path {path} has {have} frames, not {start}");
    }
    // The state hash at the start takes a replay from the nearest cached
    // start, slow on a long play with a cold cache: the clip is saved and
    // answered first, and the hash is added to its .toml when the replay
    // ends.
    let (meta, log) = clip_paths::clip_fields(path, start, &inputs);
    // The original's random draws the page saw over the clip, when it
    // recorded them (a mapping with `score_draws`): checked before
    // anything is saved.
    let draws = match v["draws"].as_str() {
        Some(t) => Some(
            gp_sim::DrawScript::parse(t)
                .map(|d| d.text())
                .map_err(|e| anyhow::anyhow!("the clip's draws: {e}"))?,
        ),
        None => None,
    };
    let (n, again) = session.add_player_clip(meta, &log, note)?;
    if let Some(d) = draws
        && !again
    {
        fs::write(session.dir.join(format!("clips/{n:03}.draws")), d)?;
    }
    let kept = format!("sealed/{}/{SEALED_ROW}/clips/{n:03}", s.game);
    // The sealed-run row hears of it, by where it is kept and not by its
    // presses.
    if sealed
        && !again
        && let Ok(row) = s.session_named(SEALED_ROW)
    {
        let text = if note.is_empty() {
            "A sealed run."
        } else {
            note
        };
        row.turn_once(json!({"by": "player", "text": text, "clips": [], "sealed": kept}))?;
    }
    if !again {
        let (session, home, rom, path) = (
            session.clone(),
            s.paths.home.clone(),
            s.rom.clone(),
            path.to_owned(),
        );
        std::thread::spawn(move || {
            let hash =
                clip_paths::original_at(&home, &rom, &path, start).and_then(|o| o.state_hex());
            match hash {
                Ok(h) => {
                    if let Err(e) = session.set_clip_field(n, "state", h.into()) {
                        eprintln!("clip {n}: the state hash wasn't saved: {e:#}");
                    }
                }
                Err(e) => eprintln!("clip {n}: no state hash: {e:#}"),
            }
        });
    }
    if sealed {
        return Ok(Response::json(
            &json!({"n": n, "again": again, "sealed": kept}),
        ));
    }
    Ok(Response::json(
        &json!({"n": n, "again": again, "mechanic": session.mechanic()}),
    ))
}

#[cfg(test)]
mod tests {

    /// A row's name is the person's first four words, lower case.
    #[test]
    fn a_row_is_named_from_the_first_words() {
        assert_eq!(
            super::row_name("The GET READY screen needs work"),
            "the-get-ready-screen"
        );
        assert_eq!(super::row_name("  !! "), "item");
    }
    use super::*;
    use crate::mechanic::session::write_queue;
    use crate::test_paths as paths;

    /// An arcade original's stage is read from its RAM as the clip plays:
    /// a made-up set whose loop copies input port 1 into E000, each value
    /// a stage, and the stick pushed on the second of three frames.
    #[test]
    fn an_arcade_clip_spans_the_stages_its_ram_byte_names() {
        let mut o = crate::original::OriginalRuntime::Arcade(
            gp_arcade::protocol::ArcadeRuntime::load(&gp_arcade::testrom::busy()),
        );
        let names: std::collections::HashMap<u8, String> =
            (0..=255u8).map(|v| (v, format!("v{v:02x}"))).collect();
        let mut inputs = gp_evidence::InputLog::new_arcade("test", "CSRLDUT");
        let pushed = gp_evidence::input_log::InputFrame::held(0x3c);
        inputs.frames = vec![
            gp_evidence::input_log::InputFrame::default(),
            pushed,
            gp_evidence::input_log::InputFrame::default(),
        ];
        let seen = stages_on(&mut o, 0xe000, &names, &inputs).unwrap();
        let v = spans_of(&seen);
        assert_eq!(v["spans"], json!(true), "{v}");
        let from: Vec<usize> = seen.iter().map(|(_, f)| *f).collect();
        assert!(from.contains(&2) && from.contains(&3), "{v}");
    }

    /// A 16KB NROM that loops in place: made up here, no game's bytes.
    fn rom() -> Vec<u8> {
        let mut rom = b"NES\x1a\x01\x01\x00\x00".to_vec();
        rom.resize(16, 0);
        let mut prg = vec![0u8; 0x4000];
        prg[..3].copy_from_slice(&[0x4C, 0x00, 0xC0]);
        prg[0x3FFA..].copy_from_slice(&[0x00, 0xC0, 0x00, 0xC0, 0x00, 0xC0]);
        rom.extend(prg);
        rom.extend(vec![0u8; 0x2000]);
        rom
    }

    /// A server over a temp home with the session `walk` open and a quest
    /// log, its page in `page/` and no player built.
    fn server(d: &Path) -> Server {
        let p = paths(d);
        let repo = d.join("repo");
        fs::create_dir_all(repo.join("mechanic")).unwrap();
        for f in [
            "index.html",
            "app.js",
            "nesplayer.js",
            "style.css",
            "refine.html",
            "refine.js",
        ] {
            fs::write(repo.join("mechanic").join(f), format!("the page's {f}")).unwrap();
        }
        let open = Arc::new(Session::create(&p, "mosshop", "walk", None, "ab").unwrap());
        write_queue(
            &d.join("mechanics/mosshop/queue.json"),
            &[
                json!({"mechanic": "walk", "note": "walking", "status": "agent working"}),
                json!({"mechanic": "fireball-moves", "note": "a fireball", "status": "next"}),
            ],
        )
        .unwrap();
        let mut srv = Server::new(p, "mosshop", &repo, rom(), "ab", 7710);
        srv.wasm = d.join("no-such/gp_play.wasm");
        *srv.session.lock().unwrap() = Some(open.clone());
        srv.open.lock().unwrap().insert("walk".into(), open);
        srv
    }

    fn call(
        srv: &Server,
        method: &str,
        target: &str,
        body: Option<Value>,
    ) -> (u16, String, String) {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let req = Request {
            method: method.into(),
            path: path.into(),
            query: query
                .split('&')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
            headers: [("host".to_owned(), "127.0.0.1:7710".to_owned())].into(),
            body: body.map(|b| b.to_string().into_bytes()).unwrap_or_default(),
        };
        match handle(srv, &req) {
            Ok(r) => (
                r.status,
                r.content_type.to_owned(),
                String::from_utf8_lossy(&r.body).into_owned(),
            ),
            Err(e) => (500, String::new(), format!("{e:#}")),
        }
    }

    fn get(srv: &Server, target: &str) -> (u16, String) {
        let (s, _, b) = call(srv, "GET", target, None);
        (s, b)
    }

    fn post(srv: &Server, target: &str, body: Value) -> (u16, Value) {
        let (s, _, b) = call(srv, "POST", target, Some(body));
        (s, serde_json::from_str(&b).unwrap_or(json!(b)))
    }

    fn log(frames: &[&str]) -> String {
        let mut s = String::from("gp-input-log 1\ncontrollers 1\n");
        for f in frames {
            s.push_str(&format!("P {f}\n"));
        }
        s
    }

    /// A clip's own set (rom?set=SET): the page's ROM when it is that set,
    /// by id or MAME system; another set not in the ROM folder, or one the
    /// game doesn't have, said in plain words; and a log's set read.
    #[test]
    fn a_clip_plays_on_its_own_set() {
        let d = tempfile::tempdir().unwrap();
        let mut srv = server(d.path());
        srv.set = "hop".into();
        srv.sets = vec![
            json!({"id": "hop", "system": "mosshop", "sha256": "ab", "here": true}),
            json!({"id": "hopg", "system": "mosshopg", "sha256": "cd", "here": false}),
        ];
        let page = call(&srv, "GET", "/rom", None).2;
        for want in ["hop", "mosshop"] {
            let (s, kind, body) = call(&srv, "GET", &format!("/rom?set={want}"), None);
            assert_eq!(
                (s, kind.as_str()),
                (200, "application/octet-stream"),
                "{want}"
            );
            assert_eq!(body, page, "{want}");
        }
        let (s, body) = get(&srv, "/rom?set=mosshopg");
        assert_eq!(s, 409, "{body}");
        assert!(body.contains("recorded on hopg, which isn't in"), "{body}");
        assert!(body.contains("gp roms add"), "{body}");
        let (s, body) = get(&srv, "/rom?set=other");
        assert_eq!(s, 404, "{body}");
        assert!(
            body.contains("recorded on other, a set mosshop doesn't have"),
            "{body}"
        );
        assert_eq!(
            log_set("gp-arcade-input 0\nsystem rooftop\ncolumns CSRLDUT\n6 .......\n").as_deref(),
            Some("rooftop")
        );
        assert_eq!(log_set(&log(&["........"])), None);
    }

    /// The page's files, the ROM, and the player's absence said plainly as
    /// JSON (the page shows the message instead of compiling it).
    #[test]
    fn the_page_the_rom_and_a_missing_player() {
        let d = tempfile::tempdir().unwrap();
        let srv = server(d.path());
        let (s, kind, body) = call(&srv, "GET", "/", None);
        assert_eq!((s, kind.as_str()), (200, "text/html; charset=utf-8"));
        assert_eq!(body, "the page's index.html");
        assert_eq!(get(&srv, "/app.js").1, "the page's app.js");
        let (s, kind, _) = call(&srv, "GET", "/rom", None);
        assert_eq!((s, kind.as_str()), (200, "application/octet-stream"));
        let (s, kind, body) = call(&srv, "GET", "/gp_play.wasm", None);
        assert_eq!((s, kind.as_str()), (404, "application/json"));
        assert!(
            body.contains("isn't built: run scripts/web/build-play.sh"),
            "{body}"
        );
        let mut srv = srv;
        srv.wasm = d.path().join("gp_play.wasm");
        fs::write(&srv.wasm, b"\0asm").unwrap();
        let (s, kind, _) = call(&srv, "GET", "/gp_play.wasm", None);
        assert_eq!((s, kind.as_str()), (200, "application/wasm"));
        assert_eq!(get(&srv, "/refine.js").1, "the page's refine.js");
        assert_eq!(get(&srv, "/refine").1, "the page's refine.html");
        // No rule found yet and no refine run: both say so as JSON.
        let rule: serde_json::Value = serde_json::from_str(&get(&srv, "/api/rule").1).unwrap();
        assert_eq!(
            (rule["mechanic"].as_str(), rule["rule"].is_null()),
            (Some("jump"), true)
        );
        let run: serde_json::Value = serde_json::from_str(&get(&srv, "/api/refine").1).unwrap();
        assert_eq!(run["run"], serde_json::Value::Null);
        assert_eq!(get(&srv, "/api/live").1, r#"{"lines":[]}"#);
    }

    /// A sealed run goes to the sealed store, outside every session, and
    /// is listed and read back only through the sealed routes; the
    /// sealed-run row hears where it is kept.
    #[test]
    fn a_sealed_run_is_kept_apart() {
        let d = tempfile::tempdir().unwrap();
        let srv = server(d.path());
        let mut q = read_queue(&d.path().join("mechanics/mosshop/queue.json"));
        q.push(json!({"mechanic": SEALED_ROW, "note": "held out", "status": "next"}));
        write_queue(&d.path().join("mechanics/mosshop/queue.json"), &q).unwrap();
        let play = log(&["........", "........", ".......A"]);
        let (_, v) = post(&srv, "/api/paths", json!({"log": play}));
        let path = v["path"].as_str().unwrap().to_owned();
        let inputs = log(&["R.......", "R......A"]);
        let (s, v) = post(
            &srv,
            "/api/clips",
            json!({"path": path, "start": 3, "inputs": inputs, "note": "a clear", "sealed": true}),
        );
        assert_eq!(s, 200, "{v}");
        assert_eq!(v["sealed"], "sealed/mosshop/sealed-run/clips/001");
        assert!(
            d.path()
                .join("sealed/mosshop/sealed-run/clips/001.toml")
                .is_file()
        );
        assert!(
            !d.path()
                .join("mechanics/mosshop/walk/clips/001.toml")
                .exists()
        );
        let (_, body) = get(&srv, "/api/sealed");
        let runs: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(runs["runs"][0]["id"], "sealed-run-001");
        let (s, full) = get(&srv, "/api/sealed/sealed-run-001.log");
        assert_eq!(s, 200);
        assert_eq!(full.lines().count(), 2 + 5, "{full}");
        let (s, body) = get(&srv, "/api/sealed/sealed-run-001.rebuild.json");
        assert_eq!(s, 409);
        assert!(body.contains("gp sealed"), "{body}");
        let row = Session::open(&srv.paths, "mosshop", SEALED_ROW).unwrap();
        assert_eq!(
            row.turns().unwrap().last().unwrap()["sealed"],
            "sealed/mosshop/sealed-run/clips/001"
        );
    }

    /// The scoring status for REVIEW REBUILD: none yet says so, and a
    /// sealed row is left out.
    #[test]
    fn the_scores_leave_sealed_rows_out() {
        let d = tempfile::tempdir().unwrap();
        let srv = server(d.path());
        assert_eq!(get(&srv, "/api/rebuild/scores").1, r#"{"status":null}"#);
        let f = d.path().join("work/refine/mosshop/status.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(
            &f,
            json!({"rows": [{"mechanic": "walk"}, {"mechanic": "sealed-run"}],
                   "clips": [{"mechanic": "sealed-run", "clip": 1}, {"mechanic": "walk", "clip": 1}]})
            .to_string(),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&get(&srv, "/api/rebuild/scores").1).unwrap();
        assert_eq!(v["status"]["rows"], json!([{"mechanic": "walk"}]));
        assert_eq!(
            v["status"]["clips"],
            json!([{"mechanic": "walk", "clip": 1}])
        );
    }

    /// The stage a clip passes through, read from observe.toml's `[level]`.
    #[test]
    fn an_observe_level_names_its_stages() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("observe.toml");
        fs::write(
            &f,
            "[level]\nnames = [[\"0x01\", \"main\"], [\"0x02\", \"m50\"]]\nram = [83]\n",
        )
        .unwrap();
        let (addr, names) = level_key(&f).unwrap();
        assert_eq!(addr, 83);
        assert_eq!(names[&2], "m50");
        assert!(level_key(&d.path().join("none.toml")).is_none());
    }

    /// The rebuild without a games checkout: each route says in words why
    /// there is nothing to play, on an arcade game the same as on the NES
    /// (an arcade game's model plays in the browser too, issue #3).
    #[test]
    fn no_rebuild_says_why() {
        let d = tempfile::tempdir().unwrap();
        let mut srv = server(d.path());
        for platform in [Platform::Nes, Platform::Arcade] {
            srv.platform = platform;
            for route in ["/api/rebuild/status", "/api/rebuild/profiles"] {
                let (s, body) = get(&srv, route);
                assert_eq!(s, 404, "{route}");
                assert!(body.contains("--games"), "{body}");
                assert!(!body.contains("draws nothing yet"), "{body}");
            }
            let (s, v) = post(&srv, "/api/rebuild/build", json!({}));
            assert_eq!(s, 404, "{v}");
        }
    }

    /// An arcade game's page: the player's absence names the arcade build,
    /// and ON THE MODEL reviews a clip as on the NES (here, with no games
    /// checkout, it says so), its review saying the model is drawn by tile
    /// kind.
    #[test]
    fn an_arcade_page_names_its_player_and_reviews_on_the_model() {
        let d = tempfile::tempdir().unwrap();
        let mut srv = server(d.path());
        srv.platform = Platform::Arcade;
        srv.wasm = d.path().join("no-such/gp_arcade_play.wasm");
        let (s, body) = get(&srv, "/gp_play.wasm");
        assert_eq!(s, 404);
        assert!(body.contains("build-play.sh --arcade"), "{body}");
        // The rebuild's board sound: the player beside the frame, and the
        // sets gp knows as the lift's sources.toml.
        assert_eq!(get(&srv, "/rebuild/gp_arcade_play.wasm").0, 404);
        fs::create_dir_all(d.path().join("no-such")).unwrap();
        fs::write(&srv.wasm, b"\0asm").unwrap();
        let (s, kind, _) = call(&srv, "GET", "/rebuild/gp_arcade_play.wasm", None);
        assert_eq!((s, kind.as_str()), (200, "application/wasm"));
        let (s, body) = get(&srv, "/rebuild/lift/sources.toml");
        assert_eq!(s, 200);
        assert_eq!(body, gp_arcade::romset::known_roms_toml());
        let play = log(&["........", "........"]);
        let (_, v) = post(&srv, "/api/paths", json!({"log": play}));
        let path = v["path"].as_str().unwrap().to_owned();
        let inputs = log(&["........"]);
        post(
            &srv,
            "/api/clips",
            json!({"path": path, "start": 1, "inputs": inputs, "mechanic": "walk"}),
        );
        for route in ["/api/clips/1/review.json", "/api/clips/1.rebuild.json"] {
            let (s, body) = get(&srv, route);
            assert_eq!(s, 404, "{route}: {body}");
            assert!(body.contains("--games"), "{route}: {body}");
        }
        assert_eq!(srv.art_said(json!({}))["art"], json!(ARCADE_ART));
        // A model that names its board's art isn't drawn by tile kind.
        let game = d.path().join("cab");
        std::fs::create_dir_all(&game).unwrap();
        let model = "[model]\nschema = \"0.4\"\nname = \"cab\"\ntitle = \"Cab\"\nsource = \"cab\"\n\
                     view = \"side\"\nframe_rate = \"30\"\nsystems = []\n\n\
                     [input]\ndevice = \"arcade\"\n\n[input.actions]\nleft = \"left\"\n\n\
                     [entities.player]\nrole = \"player\"\nsize = [16, 16]\n\n\
                     [entities.player.hitbox]\nx = 0\ny = 0\nw = 16\nh = 16\n";
        std::fs::write(game.join("model.toml"), model).unwrap();
        srv.rebuild = Some(super::super::review::Rebuild {
            game: game.clone(),
            module: None,
            rom: None,
            assets: None,
        });
        assert!(gp_model::Model::parse(model).is_ok());
        assert_eq!(srv.art_said(json!({}))["art"], json!(ARCADE_ART));
        let board = format!(
            "{model}\n[entities.player.board_sprite]\nslot = \"sprites\"\npicture = {{ number = 1 }}\n"
        );
        assert!(gp_model::Model::parse(&board).is_ok());
        std::fs::write(game.join("model.toml"), board).unwrap();
        assert_eq!(srv.art_said(json!({})), json!({}));
        srv.platform = Platform::Nes;
        assert_eq!(srv.art_said(json!({})), json!({}));
        assert!(wasm_path(Path::new("/r"), Platform::Arcade).ends_with("gp_arcade_play.wasm"));
    }

    /// The screen's routes: the agent's ask as the tool wrote it, the
    /// page's report kept and read back, and a clip's start and length.
    #[test]
    fn the_screen_reads_the_ask_and_keeps_what_the_page_shows() {
        let d = tempfile::tempdir().unwrap();
        let srv = server(d.path());
        let (s, body) = get(&srv, "/api/screen");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            (s, v["ask"].clone(), v["shown"].clone()),
            (200, Value::Null, Value::Null)
        );
        let a = gp_mcp::screen::ShowArgs {
            game: "mosshop".into(),
            view: Some("diff".into()),
            ..Default::default()
        };
        gp_mcp::screen::show_at(&srv.paths.home, &a, std::time::Duration::ZERO).unwrap();
        let v: Value = serde_json::from_str(&get(&srv, "/api/screen").1).unwrap();
        assert_eq!(
            (v["ask"]["seq"].clone(), v["ask"]["view"].clone()),
            (json!(1), json!("diff"))
        );
        let (s, r) = post(
            &srv,
            "/api/screen",
            json!({"applied": 1, "view": "diff", "by": "agent", "clip": null}),
        );
        assert_eq!(s, 200, "{r}");
        assert!(r["at_ms"].as_u64().is_some());
        let v: Value = serde_json::from_str(&get(&srv, "/api/screen").1).unwrap();
        assert_eq!(v["shown"]["applied"], 1);
        assert_eq!(post(&srv, "/api/screen", json!([1])).0, 400);
        // A clip's start and length, from the open row's clip.
        let play = log(&["........", "........", ".......A", "........"]);
        let (_, p) = post(&srv, "/api/paths", json!({"log": play}));
        // With the draws the page saw over it, kept beside it.
        let draws = "gp-draws 0\n3 pick 7\n";
        let body = json!({"path": p["path"], "start": 2, "inputs": log(&[".......A", "........"]), "mechanic": "walk", "draws": draws});
        assert_eq!(post(&srv, "/api/clips", body).0, 200);
        let v: Value = serde_json::from_str(&get(&srv, "/api/screen/clip?n=1").1).unwrap();
        assert_eq!(
            (
                v["start"].clone(),
                v["frames"].clone(),
                v["mechanic"].clone(),
                v["draws"].clone()
            ),
            (json!(2), json!(2), json!("walk"), json!(draws))
        );
        assert_eq!(get(&srv, "/api/clips/1.draws?mechanic=walk").1, draws);
        // A script that doesn't read keeps no clip.
        let bad = json!({"path": p["path"], "start": 1, "inputs": log(&["........"]), "mechanic": "walk", "draws": "nope"});
        assert_ne!(post(&srv, "/api/clips", bad).0, 200);
        assert_eq!(get(&srv, "/api/screen/clip?n=2").0, 404);
        assert_eq!(get(&srv, "/api/clips/2.draws?mechanic=walk").0, 404);
        assert_eq!(get(&srv, "/api/screen/clip?n=9").0, 404);
        assert_eq!(get(&srv, "/api/screen/clip").0, 400);
    }

    /// The core loop over HTTP: the play kept as a path, a clip on it handed
    /// to the open row with a note, its full log back, the person's turn,
    /// and the session showing both.
    #[test]
    fn a_clip_and_a_turn_reach_the_session() {
        let d = tempfile::tempdir().unwrap();
        let srv = server(d.path());
        let play = log(&["........", "........", ".......A", "........"]);
        let (s, v) = post(&srv, "/api/paths", json!({"log": play, "warm": 2}));
        assert_eq!(s, 200, "{v}");
        let base = v["path"].as_str().unwrap().to_owned();
        assert_eq!(v["frames"], 4);
        // The rest of the play, sent as a tail on the path already kept.
        let (s, v) = post(
            &srv,
            "/api/paths",
            json!({"base": base, "tail": log(&["R.......", "R......A"])}),
        );
        assert_eq!((s, v["frames"].clone()), (200, json!(6)));
        let path = v["path"].as_str().unwrap().to_owned();
        let inputs = log(&["R.......", "R......A"]);
        let body = json!({"path": path, "start": 4, "inputs": inputs, "note": "a hop", "mechanic": "walk"});
        let (s, v) = post(&srv, "/api/clips", body.clone());
        assert_eq!(s, 200, "{v}");
        assert_eq!(
            (v["n"].clone(), v["again"].clone()),
            (json!(1), json!(false))
        );
        // Pressed twice: one clip.
        assert_eq!(post(&srv, "/api/clips", body).1["again"], true);
        let (s, full) = get(&srv, "/api/clips/1.log");
        assert_eq!(s, 200);
        assert_eq!(full.lines().count(), 2 + 6, "{full}");
        assert_eq!(get(&srv, "/api/clips/9.log").0, 404);
        // The plain form: a full log and a start.
        let (s, v) = post(
            &srv,
            "/api/clips",
            json!({"log": play, "start": 1, "note": "plain"}),
        );
        assert_eq!((s, v["n"].clone()), (200, json!(2)));
        let (s, v) = post(
            &srv,
            "/api/talk",
            json!({"text": "a hop", "clips": [1], "mechanic": "walk"}),
        );
        assert_eq!((s, v["turn"].clone()), (200, json!(1)));
        let (_, body) = get(&srv, "/api/session");
        let st: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(st["clips"][0]["note"], "a hop");
        assert_eq!(st["clips"][0]["path"], path.as_str());
        assert_eq!(st["talk"][0]["clips"][0], 1);
        assert_eq!(st["waiting_for"], "agent");
        assert_eq!(st["queue"][0]["status"], "agent working");
        // The idle screen loops the newest clip of the row being worked on,
        // with no measurements.
        let (_, body) = get(&srv, "/api/working");
        let w: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(w["mechanic"], "walk");
        assert_eq!(w["clip"]["n"], 2);
        assert!(w.get("measure").is_none());
        // The state hash comes in the background.
        for _ in 0..200 {
            if st_clip_state(&srv).is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(st_clip_state(&srv).map(|h| h.len()), Some(64));
    }

    fn st_clip_state(srv: &Server) -> Option<String> {
        let st = srv.open_session().ok()?.state().ok()?;
        st["clips"][0]["state"].as_str().map(str::to_owned)
    }

    /// A clip names its mechanic: it is filed there (the session made, with
    /// no turn, when new) while another mechanic stays open, and both
    /// requests for it share one session. Picking a row opens it and hands
    /// nothing to the agent; HAND TO THE AGENT does, once.
    #[test]
    fn a_clip_goes_to_the_row_it_names_not_the_open_one() {
        let d = tempfile::tempdir().unwrap();
        let srv = server(d.path());
        let f = srv.target(Some("fireball-moves")).unwrap();
        assert!(Arc::ptr_eq(
            &f,
            &srv.target(Some("fireball-moves")).unwrap()
        ));
        assert!(f.dir.ends_with("fireball-moves"));
        assert!(f.turns().unwrap().is_empty());
        assert!(srv.target(None).unwrap().dir.ends_with("walk"));
        assert!(srv.target(Some("not-a-row")).is_err());
        let (s, v) = post(
            &srv,
            "/api/clips",
            json!({"log": log(&["........", ".......A"]), "start": 0, "mechanic": "fireball-moves"}),
        );
        assert_eq!((s, v["mechanic"].clone()), (200, json!("fireball-moves")));
        assert_eq!(srv.open_session().unwrap().clips().unwrap().len(), 0);
        let (s, v) = post(&srv, "/api/pick", json!({"mechanic": "fireball-moves"}));
        assert_eq!((s, v["mechanic"].clone()), (200, json!("fireball-moves")));
        assert!(srv.open_session().unwrap().dir.ends_with("fireball-moves"));
        assert_eq!(current(&srv.paths, "mosshop").unwrap(), "fireball-moves");
        let status = || read_queue(&srv.queue_file().unwrap())[1]["status"].clone();
        let before = status();
        assert_ne!(before, "agent working");
        assert!(f.turns().unwrap().is_empty());
        assert_eq!(post(&srv, "/api/pick", json!({"mechanic": "nope"})).0, 404);
        // HAND TO THE AGENT: the "Let's do" turn, once.
        let (s, v) = post(&srv, "/api/handoff", json!({"mechanic": "fireball-moves"}));
        assert_eq!((s, v["again"].clone()), (200, json!(false)));
        assert_eq!(status(), "agent working");
        assert_eq!(
            f.turns().unwrap()[0]["text"],
            "Let's do fireball-moves: a fireball"
        );
        let (_, v) = post(&srv, "/api/handoff", json!({"mechanic": "fireball-moves"}));
        assert_eq!(v["again"], json!(true));
        assert_eq!(f.turns().unwrap().len(), 1);
        assert_eq!(
            post(&srv, "/api/handoff", json!({"mechanic": "nope"})).0,
            404
        );
    }

    /// A POST from another origin is refused; this server's own origin
    /// and a request with none are let through.
    #[test]
    fn only_the_servers_own_page_writes() {
        let d = tempfile::tempdir().unwrap();
        let srv = server(d.path());
        let req = |origin: Option<&str>| Request {
            method: "POST".into(),
            path: "/api/talk".into(),
            headers: [("host".to_owned(), "127.0.0.1:7710".to_owned())]
                .into_iter()
                .chain(origin.map(|o| ("origin".to_owned(), o.to_owned())))
                .collect(),
            body: br#"{"text": "hi"}"#.to_vec(),
            ..Default::default()
        };
        assert_eq!(
            handle(&srv, &req(Some("http://evil.example")))
                .unwrap()
                .status,
            403
        );
        assert_eq!(
            handle(&srv, &req(Some("http://127.0.0.1:7711")))
                .unwrap()
                .status,
            403
        );
        assert_eq!(
            handle(&srv, &req(Some("http://127.0.0.1:7710")))
                .unwrap()
                .status,
            200
        );
        assert_eq!(
            handle(&srv, &req(Some("http://localhost:7710")))
                .unwrap()
                .status,
            200
        );
        assert_eq!(handle(&srv, &req(None)).unwrap().status, 200);
        // The four let through said the same words: one turn.
        assert_eq!(srv.open_session().unwrap().turns().unwrap().len(), 1);
    }

    /// The watch panel's data and a pinned file, from the session the page
    /// names; nothing else in the session folder is served.
    #[test]
    fn the_watch_panel_reads_the_agents_log_pins_and_problems() {
        let d = tempfile::tempdir().unwrap();
        let srv = server(d.path());
        let dir = srv.open_session().unwrap().dir.clone();
        super::super::watch::log(&dir, "Measuring the jump").unwrap();
        let pic = d.path().join("sheet.png");
        fs::write(&pic, b"png bytes").unwrap();
        super::super::watch::pin(&dir, &pic, "the sheet", false).unwrap();
        let (status, body) = get(&srv, "/api/watch");
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["status"]["text"], "Measuring the jump");
        assert_eq!(v["pins"][0]["file"], "001-sheet.png");
        let (status, _, b) = call(&srv, "GET", "/api/watch/files/001-sheet.png", None);
        assert_eq!((status, b.as_str()), (200, "png bytes"));
        assert_eq!(get(&srv, "/api/watch/files/..%2Fsession.toml").0, 404);
        assert_eq!(get(&srv, "/api/watch/files/../session.toml").0, 404);

        // Another topic's log, pins and open problems show too, by topic.
        let other = dir.parent().unwrap().join("emulator");
        super::super::watch::log(&other, "Booting the main Z80").unwrap();
        super::super::watch::pin(&other, &pic, "the bar", false).unwrap();
        let (_, body) = get(&srv, "/api/watch");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert!(
            v["log"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["topic"] == "emulator" && l["text"] == "Booting the main Z80"),
            "{v}"
        );
        assert_eq!(v["open"], dir.file_name().unwrap().to_str().unwrap());
        let (status, _, b) = call(
            &srv,
            "GET",
            "/api/watch/files/001-sheet.png?topic=emulator",
            None,
        );
        assert_eq!((status, b.as_str()), (200, "png bytes"));
        assert_eq!(get(&srv, "/api/watch/files/001-sheet.png?topic=..").0, 404);
    }

    /// A review asks for a clip the session has and a games checkout to
    /// play it on; its pictures are only its own.
    #[test]
    fn a_review_needs_the_clip_and_the_game() {
        let d = tempfile::tempdir().unwrap();
        let srv = server(d.path());
        assert_eq!(get(&srv, "/api/clips/9/review.json").0, 404);
        let dir = srv.open_session().unwrap().dir.clone();
        fs::write(dir.join("clips/001.toml"), "start = 0\n").unwrap();
        let (status, body) = get(&srv, "/api/clips/1/review.json");
        assert_eq!(status, 404);
        assert!(body.contains("no games checkout"), "{body}");
        assert_eq!(get(&srv, "/api/clips/1/review/meta.json").0, 404);
        assert_eq!(get(&srv, "/api/clips/1/review/../001.log").0, 404);
    }

    /// A request sent under another name (DNS rebinding: a page elsewhere
    /// pointing its own name at 127.0.0.1) is refused, reads included.
    #[test]
    fn a_request_under_another_host_name_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let srv = server(d.path());
        let req = |host: Option<&str>, path: &str| Request {
            method: "GET".into(),
            path: path.into(),
            headers: host
                .map(|h| [("host".to_owned(), h.to_owned())].into())
                .unwrap_or_default(),
            ..Default::default()
        };
        for path in ["/rom", "/api/session"] {
            for host in [Some("evil.example:7710"), Some("127.0.0.1:7711"), None] {
                assert_eq!(
                    handle(&srv, &req(host, path)).unwrap().status,
                    403,
                    "{host:?} {path}"
                );
            }
            assert_eq!(
                handle(&srv, &req(Some("localhost:7710"), path))
                    .unwrap()
                    .status,
                200
            );
        }
    }

    /// The server listens on this machine only, and answers a real
    /// connection.
    #[test]
    fn the_server_binds_localhost_only() {
        let l = listen(0).unwrap();
        let addr = l.local_addr().unwrap();
        assert!(addr.ip().is_loopback());
        let d = tempfile::tempdir().unwrap();
        let srv = Arc::new(server(d.path()));
        let s = srv.clone();
        std::thread::spawn(move || {
            let (c, _) = l.accept().unwrap();
            let _ = serve_one(&s, &c);
        });
        let mut c = TcpStream::connect(addr).unwrap();
        std::io::Write::write_all(
            &mut c,
            b"GET /api/session HTTP/1.1\r\nHost: 127.0.0.1:7710\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
        let mut out = String::new();
        std::io::Read::read_to_string(&mut c, &mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 200 OK"), "{out}");
        assert!(out.contains(r#""mechanic":"walk""#), "{out}");
    }

    /// The page's input table (mechanic/actions.test.mjs): nothing acts
    /// while a clip is sending, and a second press can't hand off twice.
    /// Skipped where there is no node.
    #[test]
    fn the_page_does_nothing_twice() {
        let test = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mechanic/actions.test.mjs");
        match std::process::Command::new("node").arg(&test).output() {
            Ok(o) => assert!(
                o.status.success(),
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
            Err(_) => eprintln!("skipped: no node"),
        }
    }

    /// With no mechanic named, the one open last, else the first row not
    /// done.
    #[test]
    fn the_mechanic_to_open() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        assert!(mechanic_to_open(&p, "mosshop", None).is_err());
        write_queue(
            &d.path().join("mechanics/mosshop/queue.json"),
            &[
                json!({"mechanic": "title", "status": "done"}),
                json!({"mechanic": "walk", "status": "next"}),
            ],
        )
        .unwrap();
        assert_eq!(mechanic_to_open(&p, "mosshop", None).unwrap(), "walk");
        Session::create(&p, "mosshop", "jump", None, "ab").unwrap();
        set_current(&p, "mosshop", "jump").unwrap();
        assert_eq!(mechanic_to_open(&p, "mosshop", None).unwrap(), "jump");
        assert_eq!(mechanic_to_open(&p, "mosshop", Some("hop")).unwrap(), "hop");
    }
}
