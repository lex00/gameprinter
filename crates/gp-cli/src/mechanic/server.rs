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
//! | `GET /api/live` | The agent's model playing live: always empty here |
//! | `POST /api/paths` | Keeps the play so far as a path |
//! | `POST /api/clips` | Saves a clip: a path, a start and its own inputs, or a full log and a start |
//! | `POST /api/talk` | The person's turn |
//! | `POST /api/pick` | Opens another mechanic of the quest log |
//! | `GET /api/clips/N/review.json`, `/api/clips/N/review/NAME.png` | A clip on the model beside the original ([`super::review`]) |
//! | `GET /api/clips/N.rebuild.json` | The same review as REVIEW REBUILD reads it: the pictures' addresses with the entities, what differs and the draws |
//! | `GET /api/clips/N.measure.json` | A clip's measurements, when `gp mechanic contacts` wrote them |
//! | `POST /api/clips/stage` | Whether a clip about to be handed off spans a stage |
//! | `GET /api/sealed`, `/api/sealed/ID.log` | The sealed runs, for the person alone; nothing is written |
//! | `GET /api/rebuild/scores` | The scoring status, `$GP_HOME/work/refine/GAME/status.json`, sealed rows left out |
//! | `GET /api/rebuild/status`, `/api/rebuild/profiles`, `POST /api/rebuild/build`, `/api/rebuild/start`, `GET /rebuild/*` | The rebuild played live ([`super::live`]) |

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use serde_json::{Value, json};

use super::http::{self, Request, Response};
use super::paths as clip_paths;
use super::session::{Session, current, queue_file, read_queue, set_current, write_queue};
use crate::Outcome;
use crate::original::Platform;

/// What `gp mechanic start` was asked.
pub(crate) struct Start<'a> {
    pub game: &'a str,
    pub mechanic: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub port: u16,
    pub games: Option<&'a Path>,
    /// The game's built module, for the review of clips on the model.
    pub module: Option<&'a Path>,
    /// The ROM, instead of the one found in $GP_ROM_DIR by hash.
    pub rom: Option<&'a Path>,
    /// Whether to restart into a newly published `$GP_HOME/bin/gp`.
    pub follow: bool,
}

/// The largest request body: a play from power-on, about 11 bytes a frame,
/// for hours.
const MAX_BODY: usize = 16 << 20;

/// The repo holding the page (`mechanic/index.html`): the working folder or
/// one above it, else the checkout gp was built from.
fn repo_root() -> Result<PathBuf> {
    let has_page = |d: &Path| d.join("mechanic/index.html").is_file();
    if let Ok(here) = std::env::current_dir()
        && let Some(d) = here.ancestors().find(|d| has_page(d))
    {
        return Ok(d.to_path_buf());
    }
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if has_page(&built) {
        return Ok(built);
    }
    bail!("no mechanic/ page found: run gp mechanic start inside the gameprinter repo")
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

/// The game's ROM: the one its roms.toml names, found by hash in
/// $GP_ROM_DIR. A source with the game's own id is taken first.
fn game_rom(
    paths: &Paths,
    games: Option<&Path>,
    game: &str,
    given: Option<&Path>,
) -> Result<crate::roms::Rom> {
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
    // The package's scored ROM first (package.toml's `role = "scored"`):
    // the clips are of that one. Then the rest in roms.toml's order.
    let scored: Vec<String> = crate::package::Package::load(&games.join(game))
        .map(|p| {
            p.entries
                .into_iter()
                .filter(|e| e.role == "scored")
                .map(|e| e.sha256)
                .collect()
        })
        .unwrap_or_default();
    let mut wanted: Vec<&gp_evidence::registry::KnownSource> = reg.sources.iter().collect();
    wanted.sort_by_key(|s| (!scored.contains(&s.sha256), s.id != game));
    let shas: Vec<String> = wanted.iter().map(|s| s.sha256.clone()).collect();
    let given: Vec<std::path::PathBuf> = given.iter().map(|p| p.to_path_buf()).collect();
    let mut found = crate::roms::find(&shas, &given, &paths.roms)?;
    for s in &shas {
        if let Some(r) = found.remove(s) {
            return Ok(r);
        }
    }
    bail!(
        "no ROM in {} that {} names: add yours with gp roms add",
        paths.roms.display(),
        file.display()
    )
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
fn published_id(link: &Path) -> Option<(std::path::PathBuf, std::time::SystemTime)> {
    let real = fs::canonicalize(link).ok()?;
    let when = fs::metadata(&real).ok()?.modified().ok()?;
    Some((real, when))
}

/// Whether `bin --version` exits 0 within ten seconds.
fn runs(bin: &Path) -> bool {
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
    use std::os::unix::process::CommandExt as _;
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
    c.args(["mechanic", "start", &srv.game, &mechanic, "--port"])
        .arg(srv.port.to_string());
    for (flag, v) in [
        ("--games", a.games),
        ("--module", a.module),
        ("--rom", a.rom),
    ] {
        if let Some(v) = v {
            c.arg(flag).arg(v);
        }
    }
    anyhow::Error::new(c.env(RESTARTED, "1").exec())
}

pub(crate) fn start(paths: &Paths, a: &Start) -> Result<Outcome> {
    let repo = repo_root()?;
    let rom = game_rom(paths, a.games, a.game, a.rom)?;
    let mechanic = mechanic_to_open(paths, a.game, a.mechanic)?;
    let listener = listen(a.port)?;
    let port = listener.local_addr()?.port();
    let platform = rom.platform;
    let mut server = Server::new(paths.clone(), a.game, &repo, rom.bytes, &rom.sha256, port);
    server.platform = platform;
    server.wasm = wasm_path(&repo, platform);
    if let Some(games) = crate::roms_cmd::games_dir(a.games) {
        server.rebuild = Some(super::review::Rebuild::new(
            paths,
            &games.join(a.game),
            a.game,
            a.module,
        ));
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
                }
            }
        });
    }
    let in_log = read_queue(&queue_file(paths, a.game)?)
        .iter()
        .any(|r| r["mechanic"] == json!(mechanic));
    // A session there already, or a quest log row (whose new session gets
    // its first turn, as a pick from the page does), else a new mechanic
    // with the person's words.
    let session = if Session::exists(paths, a.game, &mechanic)? || (in_log && a.prompt.is_none()) {
        server.session_named(&mechanic)?
    } else {
        let s = Arc::new(Session::create(
            paths,
            a.game,
            &mechanic,
            a.prompt,
            &rom.sha256,
        )?);
        server
            .open
            .lock()
            .expect("open lock")
            .insert(mechanic.clone(), s.clone());
        s
    };
    *server.session.lock().expect("session lock") = Some(session.clone());
    set_current(paths, a.game, &mechanic)?;
    let game = a.game;
    println!("{game} / {mechanic}: http://127.0.0.1:{port}/");
    println!("Session: {}", session.dir.display());
    println!(
        "Give your agent this: \"We're working on the {mechanic} of {game}. Run `gp mechanic show {game} {mechanic}` to see the session, answer with `gp mechanic say {game} {mechanic} \\\"...\\\"`, and keep the loop going: ask me for the clip you need next.\""
    );
    let _ = std::io::Write::flush(&mut std::io::stdout());
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
        }
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
    /// its "Let's do" turn, when it is new and in the quest log).
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
            let s = Session::create(&self.paths, &self.game, m, Some(&note), &self.rom_sha)?;
            s.turn(json!({"by": "player", "text": format!("Let's do {m}: {note}"), "clips": []}))?;
            s
        };
        let session = Arc::new(session);
        open.insert(m.to_owned(), session.clone());
        Ok(session)
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

    /// What the rebuild needs, or the words for why there is none.
    fn rebuild(&self) -> std::result::Result<&super::review::Rebuild, Response> {
        if self.platform == Platform::Arcade {
            return Err(Response::error(
                409,
                "the rebuild is the game's model, and an arcade game has none yet: it comes once a model of the game is written",
            ));
        }
        self.rebuild.as_ref().ok_or_else(|| {
            Response::error(
                404,
                "no games checkout, so no model to play: start the page with --games",
            )
        })
    }

    /// The session a request names with `mechanic` (the row the page had
    /// open when the person pressed A), else the one open now.
    fn target(&self, m: Option<&str>) -> Result<Arc<Session>> {
        match m {
            Some(m) => self.session_named(m),
            None => self.open_session(),
        }
    }

    /// A POST from another page is refused: only this server's own page
    /// writes.
    /// The request was sent to this machine by name: its Host is 127.0.0.1
    /// or localhost at this port. A page elsewhere that points its own name
    /// at 127.0.0.1 (DNS rebinding) sends its own name, so it can't read the
    /// ROM or the sessions.
    fn own_host(&self, req: &Request) -> bool {
        req.headers.get("host").is_some_and(|h| {
            h == &format!("127.0.0.1:{}", self.port) || h == &format!("localhost:{}", self.port)
        })
    }

    fn same_origin(&self, req: &Request) -> bool {
        match req.headers.get("origin") {
            None => true,
            Some(o) => {
                o == &format!("http://127.0.0.1:{}", self.port)
                    || o == &format!("http://localhost:{}", self.port)
            }
        }
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
    if s.own_host(&req)
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
    if !s.own_host(req) {
        return Ok(Response::error(
            403,
            "this page answers only at 127.0.0.1 or localhost on its own port",
        ));
    }
    let post = req.method == "POST";
    if post && !s.same_origin(req) {
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
        ("GET" | "HEAD", "/rom") => Ok(Response::bytes(
            "application/octet-stream",
            s.rom.as_ref().clone(),
        )),
        ("GET" | "HEAD", "/api/session") => Ok(Response::json(&s.open_session()?.state()?)),
        ("GET" | "HEAD", "/api/working") => working(s, req.param("mechanic")),
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
            if s.platform == Platform::Arcade {
                return Ok(Response::error(
                    409,
                    "ON THE MODEL plays a clip on the game's model, and an arcade game has none yet: it comes once a model of the game is written",
                ));
            }
            let Some(r) = &s.rebuild else {
                return Ok(Response::error(
                    404,
                    "no games checkout, so no model to review on: start the page with --games",
                ));
            };
            match super::review::review(&s.paths, r, &session.dir, n) {
                Ok(v) => Ok(Response::json(&v)),
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
        // What the agent is doing, for the watch panel ([`super::watch`]).
        ("GET" | "HEAD", "/api/watch") => Ok(Response::json(&super::watch::view(
            &s.target(req.param("mechanic"))?.dir,
        ))),
        ("GET" | "HEAD", p) if p.starts_with("/api/watch/files/") => {
            let name = &p["/api/watch/files/".len()..];
            match super::watch::file(&s.target(req.param("mechanic"))?.dir, name) {
                Some((kind, bytes)) => Ok(Response::bytes(kind, bytes)),
                None => Ok(Response::error(404, "no such pinned file")),
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
            let (n, again) = s.target(v["mechanic"].as_str())?.turn_once(t)?;
            Ok(Response::json(&json!({"turn": n, "again": again})))
        }
        ("POST", "/api/pick") => {
            // The person picked a mechanic from the quest log: open its
            // session (made if new) and hand it to the agent.
            let v: Value = serde_json::from_slice(&req.body)?;
            let m = v["mechanic"].as_str().context("mechanic")?;
            let qf = s.queue_file()?;
            let mut q = read_queue(&qf);
            let Some(row) = q.iter().find(|r| r["mechanic"] == json!(m)).cloned() else {
                return Ok(Response::error(404, "not in the quest log"));
            };
            let session = s.session_named(m)?;
            // Picking a new one makes it the agent's. Others keep their
            // status: one with clips handed in stays the agent's until it
            // answers, whatever the person opens next.
            if row["status"] == json!("next") {
                if let Some(r) = q.iter_mut().find(|r| r["mechanic"] == json!(m)) {
                    r["status"] = json!("agent working");
                }
                write_queue(&qf, &q)?;
            }
            if session.turns()?.is_empty() {
                let note = row["note"].as_str().unwrap_or("");
                session.turn(
                    json!({"by": "player", "text": format!("Let's do {m}: {note}"), "clips": []}),
                )?;
            }
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
                Ok(v) => Ok(Response::json(&super::review::as_rebuild(
                    &v,
                    n,
                    session.mechanic(),
                ))),
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
        ("GET" | "HEAD", "/api/rebuild/status") => match s.rebuild() {
            Ok(rb) => Ok(Response::json(&super::live::status(
                &s.paths, &s.repo, &s.game, rb,
            ))),
            Err(r) => Ok(r),
        },
        ("GET" | "HEAD", "/api/rebuild/profiles") => match s.rebuild() {
            Ok(rb) => Ok(Response::json(&super::live::profiles(
                &s.paths, &s.game, rb,
            )?)),
            Err(r) => Ok(r),
        },
        ("POST", "/api/rebuild/build") => match s.rebuild() {
            Ok(rb) => {
                if let Err(e) = super::live::build(&s.paths, &s.repo, &s.game, rb, &s.rom) {
                    return Ok(Response::error(409, &format!("{e:#}")));
                }
                Ok(Response::json(&super::live::status(
                    &s.paths, &s.repo, &s.game, rb,
                )))
            }
            Err(r) => Ok(r),
        },
        ("POST", "/api/rebuild/start") => match s.rebuild() {
            Ok(rb) => {
                let v: Value = serde_json::from_slice(&req.body)?;
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
        ("GET" | "HEAD", p) if p.starts_with("/rebuild/") => {
            super::live::file(&s.paths, &s.repo, &s.game, &p["/rebuild/".len()..])
        }
        _ => Ok(Response::error(404, "not found")),
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
/// one to another during it, as a clear does. NES only: an arcade game's
/// observe.toml names no stage yet.
fn stage_span(
    s: &Server,
    path: &str,
    start: usize,
    inputs: &gp_evidence::InputLog,
) -> Result<Value> {
    let Some(games) = s.rebuild.as_ref().map(|r| r.game.clone()) else {
        return Ok(json!({"spans": false, "why": "no games checkout"}));
    };
    if s.platform == Platform::Arcade {
        return Ok(json!({"spans": false, "why": "an arcade game's stage isn't read yet"}));
    }
    let Some((addr, names)) = level_key(&games.join("observe.toml")) else {
        return Ok(json!({"spans": false, "why": "the game's observe.toml names no stage"}));
    };
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
    Ok(json!({
        "spans": seen.len() >= 2,
        "stages": seen.iter().map(|(l, f)| json!({"stage": l, "from": f})).collect::<Vec<_>>(),
    }))
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
    let (n, again) = session.add_player_clip(meta, &log, note)?;
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
    use super::*;

    fn paths(home: &Path) -> Paths {
        Paths {
            home: home.to_path_buf(),
            store: home.join("store"),
            roms: home.join("roms"),
        }
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
        let open = Arc::new(Session::create(&p, "dk", "walk", None, "ab").unwrap());
        write_queue(
            &d.join("mechanics/dk/queue.json"),
            &[
                json!({"mechanic": "walk", "note": "walking", "status": "agent working"}),
                json!({"mechanic": "fireball-moves", "note": "a fireball", "status": "next"}),
            ],
        )
        .unwrap();
        let mut srv = Server::new(p, "dk", &repo, rom(), "ab", 7710);
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
        let mut q = read_queue(&d.path().join("mechanics/dk/queue.json"));
        q.push(json!({"mechanic": SEALED_ROW, "note": "held out", "status": "next"}));
        write_queue(&d.path().join("mechanics/dk/queue.json"), &q).unwrap();
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
        assert_eq!(v["sealed"], "sealed/dk/sealed-run/clips/001");
        assert!(
            d.path()
                .join("sealed/dk/sealed-run/clips/001.toml")
                .is_file()
        );
        assert!(!d.path().join("mechanics/dk/walk/clips/001.toml").exists());
        let (_, body) = get(&srv, "/api/sealed");
        let runs: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(runs["runs"][0]["id"], "sealed-run-001");
        let (s, full) = get(&srv, "/api/sealed/sealed-run-001.log");
        assert_eq!(s, 200);
        assert_eq!(full.lines().count(), 2 + 5, "{full}");
        let (s, body) = get(&srv, "/api/sealed/sealed-run-001.rebuild.json");
        assert_eq!(s, 409);
        assert!(body.contains("gp sealed"), "{body}");
        let row = Session::open(&srv.paths, "dk", SEALED_ROW).unwrap();
        assert_eq!(
            row.turns().unwrap().last().unwrap()["sealed"],
            "sealed/dk/sealed-run/clips/001"
        );
    }

    /// The scoring status for REVIEW REBUILD: none yet says so, and a
    /// sealed row is left out.
    #[test]
    fn the_scores_leave_sealed_rows_out() {
        let d = tempfile::tempdir().unwrap();
        let srv = server(d.path());
        assert_eq!(get(&srv, "/api/rebuild/scores").1, r#"{"status":null}"#);
        let f = d.path().join("work/refine/dk/status.json");
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

    /// The rebuild on an arcade game, and without a games checkout: each
    /// route says in words why there is nothing to play.
    #[test]
    fn no_rebuild_says_why() {
        let d = tempfile::tempdir().unwrap();
        let mut srv = server(d.path());
        let (s, body) = get(&srv, "/api/rebuild/status");
        assert_eq!(s, 404);
        assert!(body.contains("--games"), "{body}");
        srv.platform = Platform::Arcade;
        for route in ["/api/rebuild/status", "/api/rebuild/profiles"] {
            let (s, body) = get(&srv, route);
            assert_eq!(s, 409, "{route}");
            assert!(body.contains("an arcade game has none yet"), "{body}");
        }
        let (s, v) = post(&srv, "/api/rebuild/build", json!({}));
        assert_eq!(s, 409, "{v}");
    }

    /// An arcade game's page: the player's absence names the arcade build,
    /// and ON THE MODEL says in words why it can't play a clip yet.
    #[test]
    fn an_arcade_page_names_its_player_and_has_no_model_yet() {
        let d = tempfile::tempdir().unwrap();
        let mut srv = server(d.path());
        srv.platform = Platform::Arcade;
        srv.wasm = d.path().join("no-such/gp_arcade_play.wasm");
        let (s, body) = get(&srv, "/gp_play.wasm");
        assert_eq!(s, 404);
        assert!(body.contains("build-play.sh --arcade"), "{body}");
        let play = log(&["........", "........"]);
        let (_, v) = post(&srv, "/api/paths", json!({"log": play}));
        let path = v["path"].as_str().unwrap().to_owned();
        let inputs = log(&["........"]);
        post(
            &srv,
            "/api/clips",
            json!({"path": path, "start": 1, "inputs": inputs, "mechanic": "walk"}),
        );
        let (s, body) = get(&srv, "/api/clips/1/review.json");
        assert_eq!(s, 409, "{body}");
        assert!(body.contains("an arcade game has none yet"), "{body}");
        assert!(wasm_path(Path::new("/r"), Platform::Arcade).ends_with("gp_arcade_play.wasm"));
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

    /// A clip names its mechanic: it is filed there (the session made with
    /// its first turn when new) while another mechanic stays open, and both
    /// requests for it share one session. Picking a row opens it.
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
        assert_eq!(
            f.turns().unwrap()[0]["text"],
            "Let's do fireball-moves: a fireball"
        );
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
        assert_eq!(current(&srv.paths, "dk").unwrap(), "fireball-moves");
        assert_eq!(
            read_queue(&srv.queue_file().unwrap())[1]["status"],
            "agent working"
        );
        assert_eq!(post(&srv, "/api/pick", json!({"mechanic": "nope"})).0, 404);
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
        assert!(mechanic_to_open(&p, "dk", None).is_err());
        write_queue(
            &d.path().join("mechanics/dk/queue.json"),
            &[
                json!({"mechanic": "title", "status": "done"}),
                json!({"mechanic": "walk", "status": "next"}),
            ],
        )
        .unwrap();
        assert_eq!(mechanic_to_open(&p, "dk", None).unwrap(), "walk");
        Session::create(&p, "dk", "jump", None, "ab").unwrap();
        set_current(&p, "dk", "jump").unwrap();
        assert_eq!(mechanic_to_open(&p, "dk", None).unwrap(), "jump");
        assert_eq!(mechanic_to_open(&p, "dk", Some("hop")).unwrap(), "hop");
    }
}
