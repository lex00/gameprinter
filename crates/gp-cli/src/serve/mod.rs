//! `gp serve`: the editor's server (game-printer's docs/show.md, "The main
//! screen", and docs/inspector.md). It serves the editor (`editor/dist`,
//! built by `just editor`) at `/` and the routes the editor calls under
//! `/api/`, on 127.0.0.1 only.
//!
//! Ported from game-printer's `crates/gp-cli/src/serve/`. What gameprinter's
//! layout changed:
//!
//! - A run is a game of the games checkout (`games/GAME`), named by the
//!   game's folder, where game-printer's was a capture run under
//!   `$GP_HOME/capture/`. The model, the mapping and the corpus are the
//!   game's own; what the server writes for a run (the background work's
//!   state, the marks on the tool's clips) is in
//!   `$GP_HOME/work/serve/GAME/`, never in the games checkout.
//! - The clips the tool made are the game's corpus clips (`corpus.toml`
//!   and `inputs/`), each from its own start.
//! - The rebuilt game is the game's interpreter preview with its module,
//!   built and served as the mechanic page builds it
//!   ([`crate::mechanic::live`]).
//! - The HTTP is the mechanic server's ([`crate::mechanic::http`]).
//!
//! Endpoints come from a route table: each module has a `routes()`
//! function and [`MODULES`] lists them. `GET /api` lists every route with
//! what it does.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use serde_json::json;

use crate::Outcome;
use crate::mechanic::http::{self, ReadError, Request, Response};

mod annotations;
pub(crate) mod background;
pub(crate) mod clip_agent;
mod coverage;
mod edits;
pub(crate) mod freeplay;
pub(crate) mod game;
pub(crate) mod generated;
mod grid;
mod labels;
mod ladder;
mod later;
mod laya;
mod lockstep;
mod next;
mod pages;
mod pair;
mod play;
mod point;
mod pointing;
mod recordings;
mod resolve;
pub(crate) mod scenarios;
mod select;
pub(crate) mod tell;
mod tiles;
mod variants;
pub(crate) mod work;

/// Every module's routes, in the order they're matched.
const MODULES: &[fn() -> Vec<Route>] = &[
    game::routes,
    play::routes,
    freeplay::routes,
    scenarios::routes,
    generated::routes,
    background::routes,
    grid::routes,
    pages::routes,
    work::routes,
    tell::routes,
    clip_agent::routes,
    // The inspector's routes (wave 8): the recordings and their views, the
    // game view's lockstep, moments, scenario ladder and point and tell.
    recordings::routes,
    labels::routes,
    annotations::routes,
    select::routes,
    resolve::routes,
    variants::routes,
    coverage::routes,
    pair::routes,
    tiles::routes,
    lockstep::routes,
    ladder::routes,
    point::routes,
    edits::routes,
    later::routes,
    laya::routes,
    next::routes,
];

/// How many connections are served at once; more wait in the listen queue.
const MAX_CONNECTIONS: usize = 64;

/// The largest request body: a branch's frames come in pieces of 2,000
/// lines, about 22 KB, and a save point's picture is a small PNG.
const MAX_BODY: usize = 4 << 20;

/// The port `gp serve` listens on unless told otherwise.
pub(crate) const DEFAULT_PORT: u16 = 7710;

/// What `gp serve` was asked.
#[derive(clap::Args, Clone, Debug)]
pub struct ServeOpts {
    /// The address to listen on, which must be `127.0.0.1`. The server has
    /// no authentication, so it must not be reachable from another machine.
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,
    /// The port; 0 picks a free one. The URL is printed once listening.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    /// The games checkout whose games are the runs (default:
    /// $GP_GAMES_DIR, else a folder of games at or above the working
    /// directory, or its `games/`).
    #[arg(long)]
    games: Option<PathBuf>,
    /// The built editor to serve at `/`. Defaults to editor/dist in the
    /// repo, which `just editor` builds.
    #[arg(long)]
    editor: Option<PathBuf>,
    /// A game's built module, as GAME=FILE, for a game with hooks (default:
    /// `$GP_HOME/work/modules/GAME.wasm`).
    #[arg(long, value_name = "GAME=FILE")]
    module: Vec<String>,
    /// The clip agent's command and its arguments ("claude --model
    /// sonnet"), else $GP_CLIP_AGENT. Off without either: nothing starts
    /// Claude unasked (`serve/clip_agent.rs`).
    #[arg(long, value_name = "COMMAND")]
    clip_agent: Option<String>,
    /// A game's ROM set, as GAME=SET or GAME=FILE (tapper=rbtapper): the
    /// set its page plays, kept for the game as the page's set picker keeps
    /// it.
    #[arg(long, value_name = "GAME=SET")]
    rom: Vec<String>,
}

pub(crate) fn run(opts: &ServeOpts, paths: &Paths) -> Result<Outcome> {
    let ip = loopback_only(&opts.bind)?;
    let ctx = Ctx::new(paths, opts)?;
    clip_agent::set_flag(opts.clip_agent.clone());
    let listener = TcpListener::bind(SocketAddr::new(ip, opts.port))
        .with_context(|| format!("listening on {}:{}", opts.bind, opts.port))?;
    let addr = listener.local_addr()?;
    let ctx = Arc::new(Ctx {
        port: addr.port(),
        ..ctx
    });
    println!(
        "gp serve: http://{addr}/ with the games in {} ({})",
        ctx.games.display(),
        match &ctx.editor {
            Some(dir) => format!("the editor from {}", dir.display()),
            None => "the editor isn't built; `just editor` builds it".into(),
        }
    );
    use std::io::Write as _;
    std::io::stdout().flush()?;
    background::start_dispatcher(ctx.clone());
    serve(listener, ctx);
    Ok(Outcome::ok(json!({}), String::new()))
}

/// The address to bind: 127.0.0.1, or an error for anything else.
fn loopback_only(bind: &str) -> Result<IpAddr> {
    let ip: IpAddr = bind.parse().with_context(|| {
        format!("--bind {bind:?} isn't an IP address; gp serve listens on 127.0.0.1 only")
    })?;
    if ip != IpAddr::V4(Ipv4Addr::LOCALHOST) {
        bail!(
            "gp serve listens on 127.0.0.1 only, not {bind}: it has no authentication, and a game's ROM and clips are private"
        );
    }
    Ok(ip)
}

/// What every handler can see.
pub(crate) struct Ctx {
    pub paths: Paths,
    /// The gameprinter checkout: the preview's scripts and its build.
    pub repo: PathBuf,
    /// The built editor, if found.
    pub editor: Option<PathBuf>,
    /// The games checkout; each game in it is a run.
    pub games: PathBuf,
    /// Modules given with `--module`, by game.
    pub modules: HashMap<String, PathBuf>,
    pub routes: Vec<Route>,
    /// The port listened on, for the same-origin check.
    pub port: u16,
}

impl Ctx {
    /// GP_SERVE_MODULES for a command run on the server's behalf: the
    /// modules it was given, as GAME=FILE,...
    pub fn modules_env(&self) -> String {
        self.modules
            .iter()
            .map(|(g, f)| format!("{g}={}", f.display()))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The context `gp agent` reads a run with: the games checkout, no
    /// editor and no port.
    pub fn for_cli(paths: &Paths, games: Option<&Path>) -> Result<Self> {
        let games = crate::roms_cmd::games_dir(games)
            .context("no games checkout: pass --games or set GP_GAMES_DIR")?;
        // The modules `gp serve` was given, passed to the clip agent's
        // commands as GP_SERVE_MODULES (GAME=FILE, comma-separated).
        let modules = std::env::var("GP_SERVE_MODULES")
            .unwrap_or_default()
            .split(',')
            .filter_map(|m| m.split_once('='))
            .map(|(g, f)| (g.to_owned(), PathBuf::from(f)))
            .collect();
        Ok(Self {
            paths: paths.clone(),
            repo: repo_root(),
            editor: None,
            games,
            modules,
            routes: Vec::new(),
            port: 0,
        })
    }

    pub fn new(paths: &Paths, opts: &ServeOpts) -> Result<Self> {
        let repo = repo_root();
        let editor = match &opts.editor {
            Some(dir) if dir.join("index.html").is_file() => Some(dir.clone()),
            Some(dir) => bail!(
                "{} has no index.html; `just editor` builds the editor",
                dir.display()
            ),
            None => Some(repo.join("editor/dist")).filter(|d| d.join("index.html").is_file()),
        };
        let games = crate::roms_cmd::games_dir(opts.games.as_deref())
            .context("no games checkout: pass --games or set GP_GAMES_DIR")?;
        let mut modules = HashMap::new();
        for m in &opts.module {
            let (game, file) = m
                .split_once('=')
                .with_context(|| format!("--module {m}: give it as GAME=FILE"))?;
            modules.insert(game.to_owned(), PathBuf::from(file));
        }
        for r in &opts.rom {
            let (game, set) = r
                .split_once('=')
                .with_context(|| format!("--rom {r}: give it as GAME=SET or GAME=FILE"))?;
            crate::mechanic::server::choose_rom(paths, Some(&games), game, set)?;
        }
        Ok(Self {
            paths: paths.clone(),
            repo,
            editor,
            games,
            modules,
            routes: MODULES.iter().flat_map(|m| m()).collect(),
            port: opts.port,
        })
    }
}

/// The gameprinter checkout: the working folder or one above it that holds
/// `editor/` and `scripts/web/`, else the checkout gp was built from.
pub(crate) fn repo_root() -> PathBuf {
    crate::repo::find_repo(|d| d.join("scripts/web/page/preview.js").is_file())
        .unwrap_or_else(crate::repo::built)
}

/// A request handler: the context, the request and the path's `{name}`
/// parameters.
pub(crate) type Handler = fn(&Ctx, &Request, &Params) -> Result<Response, Fail>;

/// The `{name}` parts of a matched path.
pub(crate) type Params = HashMap<&'static str, String>;

/// One endpoint.
pub(crate) struct Route {
    /// A path such as `/api/play/free/{session}/saves/{save}/state`. A
    /// `{name}` part matches one path segment; a final `{*name}` matches
    /// the rest of the path.
    pub path: &'static str,
    /// What it serves, for `GET /api`.
    pub about: &'static str,
    pub handler: Handler,
}

/// A handler's failure: a status and its words.
#[derive(Debug)]
pub(crate) struct Fail {
    pub status: u16,
    pub message: String,
}

impl Fail {
    pub fn bad_request(m: impl Into<String>) -> Self {
        Self {
            status: 400,
            message: m.into(),
        }
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self {
            status: 404,
            message: m.into(),
        }
    }
    pub fn conflict(m: impl Into<String>) -> Self {
        Self {
            status: 409,
            message: m.into(),
        }
    }
}

impl From<anyhow::Error> for Fail {
    fn from(e: anyhow::Error) -> Self {
        Self {
            status: 500,
            message: format!("{e:#}"),
        }
    }
}

impl From<std::io::Error> for Fail {
    fn from(e: std::io::Error) -> Self {
        Self {
            status: 500,
            message: e.to_string(),
        }
    }
}

/// Only POST.
pub(crate) fn post_only(req: &Request) -> Result<(), Fail> {
    if req.method == "POST" {
        Ok(())
    } else {
        Err(Fail {
            status: 405,
            message: "this endpoint takes POST".into(),
        })
    }
}

/// A JSON body.
pub(crate) fn body<T: for<'de> serde::Deserialize<'de>>(req: &Request) -> Result<T, Fail> {
    serde_json::from_slice(&req.body).map_err(|e| Fail::bad_request(format!("the body: {e}")))
}

/// Matches `path` against a route's pattern.
fn matches(pattern: &'static str, path: &str) -> Option<Params> {
    let mut params = Params::new();
    let mut want = pattern.trim_start_matches('/').split('/');
    let mut got = path.trim_start_matches('/').split('/');
    loop {
        match (want.next(), got.next()) {
            (None, None) => return Some(params),
            (Some(w), Some(g)) => {
                if let Some(name) = w.strip_prefix("{*").and_then(|n| n.strip_suffix('}')) {
                    let rest: Vec<&str> = std::iter::once(g).chain(got).collect();
                    params.insert(leak(name), rest.join("/"));
                    return Some(params);
                }
                if let Some(name) = w.strip_prefix('{').and_then(|n| n.strip_suffix('}')) {
                    if g.is_empty() {
                        return None;
                    }
                    params.insert(leak(name), http::percent_decode(g)?);
                } else if w != g {
                    return None;
                }
            }
            _ => return None,
        }
    }
}

/// A route's parameter name, which is one of the patterns' own `'static`
/// strings; found again by value so no new string is leaked.
fn leak(name: &str) -> &'static str {
    const NAMES: &[&str] = &[
        "run", "session", "branch", "save", "clip", "start", "sha256", "file", "name", "n", "id",
        "frame", "chunk", "note", "game", "copy", "seq", "which",
    ];
    NAMES
        .iter()
        .find(|n| **n == name)
        .copied()
        .unwrap_or("param")
}

fn handle(ctx: &Ctx, req: &Request) -> Response {
    let own_host = req.headers.get("host").is_some_and(|h| {
        h == &format!("127.0.0.1:{}", ctx.port) || h == &format!("localhost:{}", ctx.port)
    });
    if !own_host {
        return Response::error(
            403,
            "gp serve answers only at 127.0.0.1 or localhost on its own port",
        );
    }
    if req.method != "GET" && req.method != "HEAD" {
        let same = req.headers.get("origin").is_none_or(|o| {
            o == &format!("http://127.0.0.1:{}", ctx.port)
                || o == &format!("http://localhost:{}", ctx.port)
        });
        if !same {
            return Response::error(403, "only gp serve's own page writes");
        }
    }
    if req.path == "/api" {
        let list: Vec<_> = ctx
            .routes
            .iter()
            .map(|r| json!({"path": r.path, "about": r.about}))
            .collect();
        return Response::json(&json!({"routes": list}));
    }
    for r in &ctx.routes {
        if let Some(params) = matches(r.path, &req.path) {
            return match (r.handler)(ctx, req, &params) {
                Ok(res) => res,
                Err(f) => Response::error(f.status, &f.message),
            };
        }
    }
    if req.path.starts_with("/api/") {
        return Response::error(404, "not here");
    }
    if req.path == "/" {
        return landing(ctx, req);
    }
    static_file(ctx, &req.path)
}

/// `/`: the page of a game, at /mechanic/GAME/ with its tab kept from
/// `?tab=`: `?run=GAME`, else the game whose page was opened last
/// ($GP_HOME/work/serve/last-game), else a list of the games to pick from.
/// The editor's older screens stay at /index.html.
fn landing(ctx: &Ctx, req: &Request) -> Response {
    let runs = game::all_runs(ctx);
    let run = req
        .param("run")
        .filter(|r| runs.iter().any(|x| x == r))
        .map(str::to_owned)
        .or_else(|| pages::last_game(ctx));
    let tab = req
        .param("tab")
        .filter(|t| t.bytes().all(|b| b.is_ascii_lowercase()))
        .map(|t| format!("?tab={t}"))
        .unwrap_or_default();
    let head = "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\"><title>Game Printer</title><style>body{margin:0;background:#000;color:#fcfcfc;font:15px/1.45 ui-monospace,\"SF Mono\",Menlo,monospace}header{padding:10px 16px;border-bottom:2px solid #fcfcfc}#logo{color:#f83800;font-weight:700}main{padding:12px 16px}a{color:#fcfcfc}h2{font-size:14px;color:#f8b800}</style>";
    let Some(run) = run else {
        let items: String = runs
            .iter()
            .map(|r| format!("<li><a href=\"/mechanic/{r}/{tab}\">{r}</a></li>"))
            .collect();
        let body = if runs.is_empty() {
            format!(
                "<p>No game here yet. Ask your agent to bring in your game, and say where the file is.</p><p style=\"color:#bcbcbc\">The games folder: {}</p>",
                ctx.games.display()
            )
        } else {
            format!("<h2>Pick a game</h2><ul id=\"games\">{items}</ul>")
        };
        return Response::bytes(
            "text/html; charset=utf-8",
            format!("{head}<header><span id=logo>GAME PRINTER</span></header><main style=\"display:block\">{body}</main>")
                .into_bytes(),
        );
    };
    let to = format!("/mechanic/{run}/{tab}");
    Response::bytes(
        "text/html; charset=utf-8",
        format!("{head}<script>location.replace({to:?})</script><p><a href=\"{to}\">{run}</a></p>")
            .into_bytes(),
    )
}

/// The editor's own files.
fn static_file(ctx: &Ctx, path: &str) -> Response {
    let Some(dir) = &ctx.editor else {
        return Response::bytes(
            "text/plain; charset=utf-8",
            b"The editor isn't built: run `just editor` (npm ci and npm run build in editor/), then reload.\n".to_vec(),
        );
    };
    let name = path.trim_start_matches('/');
    let name = if name.is_empty() { "index.html" } else { name };
    if name
        .split('/')
        .any(|p| p.is_empty() || p == ".." || p.starts_with('.'))
    {
        return Response::error(404, "not found");
    }
    let kind = match Path::new(name).extension().and_then(|x| x.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    };
    match std::fs::read(dir.join(name)) {
        Ok(b) => Response::bytes(kind, b),
        Err(_) => Response::error(404, "not found"),
    }
}

fn serve_one(ctx: &Ctx, conn: &TcpStream) -> std::io::Result<()> {
    let req = match http::read_request_up_to(conn, MAX_BODY) {
        Ok(r) => r,
        Err(ReadError::Bad(status, why)) => {
            return http::write_response(conn, &Response::error(status, why), false);
        }
        Err(ReadError::Gone) => return Ok(()),
    };
    if let Some(run) = req
        .path
        .strip_prefix("/api/capture/")
        .and_then(|p| p.strip_suffix("/stream"))
        .filter(|r| game::is_name(r))
        && game::run_named(ctx, run).is_ok()
        && req.headers.get("host").is_some_and(|h| {
            h == &format!("127.0.0.1:{}", ctx.port) || h == &format!("localhost:{}", ctx.port)
        })
    {
        let after = req
            .param("after")
            .or_else(|| req.headers.get("last-event-id").map(String::as_str))
            .and_then(|a| a.parse().ok())
            .unwrap_or(0);
        return stream(conn, run, after);
    }
    let res = handle(ctx, &req);
    http::write_response(conn, &res, req.method == "HEAD")
}

/// `GET /api/capture/{run}/stream?after=SEQ`: the run's events as
/// server-sent events (each `id:` its seq), held open; a comment line every
/// 15 seconds keeps it alive. Ends when the page goes away.
fn stream(mut conn: &TcpStream, run: &str, mut after: u64) -> std::io::Result<()> {
    use std::io::Write as _;
    conn.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\nConnection: close\r\n\r\n",
    )?;
    conn.flush()?;
    loop {
        let events = background::wait_events(run, after, std::time::Duration::from_secs(15));
        if events.is_empty() {
            conn.write_all(b": still here\n\n")?;
        }
        for e in events {
            after = e["seq"].as_u64().unwrap_or(after);
            write!(conn, "id: {after}\ndata: {e}\n\n")?;
        }
        conn.flush()?;
    }
}

fn serve(listener: TcpListener, ctx: Arc<Ctx>) {
    let open = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        while open.load(std::sync::atomic::Ordering::SeqCst) >= MAX_CONNECTIONS {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        open.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (ctx, open) = (ctx.clone(), open.clone());
        std::thread::spawn(move || {
            let _ = conn.set_read_timeout(Some(std::time::Duration::from_secs(30)));
            let _ = conn.set_write_timeout(Some(std::time::Duration::from_secs(30)));
            let _ = serve_one(&ctx, &conn);
            open.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        });
    }
}

/// A request to a `gp serve` on this machine: the status and the body.
pub(crate) fn call(port: u16, method: &str, path: &str, body: &str) -> Result<(u16, String)> {
    use std::io::{Read as _, Write as _};
    let mut s = TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(120)))?;
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = out.split_once("\r\n\r\n").map_or("", |(_, b)| b).to_owned();
    Ok((status, body))
}

/// A `gp serve` answering on `port`, started in the background when none
/// does: this gp, its output in `$GP_HOME/work/serve/serve.log`. Waits up
/// to half a minute for it to answer.
pub(crate) fn ensure(paths: &Paths, games: Option<&Path>, port: u16) -> Result<()> {
    if call(port, "GET", "/api", "").is_ok_and(|(s, _)| s == 200) {
        return Ok(());
    }
    let dir = paths.home.join("work").join("serve");
    std::fs::create_dir_all(&dir)?;
    let log = std::fs::File::create(dir.join("serve.log"))?;
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg("serve").arg("--port").arg(port.to_string());
    if let Some(g) = games {
        cmd.arg("--games").arg(g);
    }
    cmd.stdout(log.try_clone()?)
        .stderr(log)
        .stdin(std::process::Stdio::null())
        .spawn()
        .context("starting gp serve")?;
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if call(port, "GET", "/api", "").is_ok_and(|(s, _)| s == 200) {
            return Ok(());
        }
    }
    bail!(
        "gp serve didn't answer on port {port}: see {}",
        dir.join("serve.log").display()
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn patterns_take_their_parts() {
        let p = matches(
            "/api/play/free/{session}/saves/{save}/state",
            "/api/play/free/free-1/saves/3/state",
        )
        .unwrap();
        assert_eq!(p["session"], "free-1");
        assert_eq!(p["save"], "3");
        assert!(matches("/api/play/free/{session}", "/api/play/free/").is_none());
        assert!(matches("/api/play/free/{session}", "/api/play/free/a/b").is_none());
        let p = matches(
            "/api/capture/{run}/preview/{*file}",
            "/api/capture/dk/preview/a/b.js",
        )
        .unwrap();
        assert_eq!(p["file"], "a/b.js");
        assert_eq!(
            matches("/api/capture/{run}/game", "/api/capture/donkey%2Dkong/game").unwrap()["run"],
            "donkey-kong"
        );
    }

    #[test]
    fn only_loopback_is_bound() {
        assert!(loopback_only("127.0.0.1").is_ok());
        assert!(loopback_only("0.0.0.0").is_err());
        assert!(loopback_only("nope").is_err());
    }
}
