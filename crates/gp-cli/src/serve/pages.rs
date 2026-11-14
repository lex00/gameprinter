//! The mechanic page and the watch panel inside the editor: one app at one
//! URL. `gp serve` mounts the mechanic page's server
//! ([`crate::mechanic::server`]) at `/mechanic/GAME/` and the watch panel's
//! ([`crate::mechanic::watch_page`]) at `/watch/GAME/TOPIC/`, each made the
//! first time it is asked for and kept, and answers every request under
//! them as their own servers did (the pages' URLs are relative, so they
//! work at either place). The editor shows them in its tabs and its side
//! panel; `gp mechanic start` and `gp mechanic watch` open them here.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET`, `POST /mechanic/{run}/...` | The mechanic page for the game and its routes (HELP WITH MECHANICS, REVIEW REBUILD, FREE PLAY on the model, the quest log, clips handed off, talk) |
//! | `GET /api/mechanic/{run}/ready` | `{ready, seconds?, failed?}`: whether the game's page is made; asking starts making it, on a thread of its own, since the first time after a new gp it builds the game's code. Until then `/mechanic/{run}/` answers at once with a page that says so and loads the game when it is ready |
//! | `POST /api/mechanic/{run}/open` | `{mechanic?, prompt?}`: opens a mechanic in the page, its session made when new |
//! | `GET /api/mechanic/{run}` | The mechanic open now and its session folder |
//! | `GET`, `POST /watch/{run}/{name}/...` | The watch panel of the session `name`: the agent's log, pins and problem rows, and notes to the agent |

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};
use crate::mechanic::server::{self as mserver, Server};
use crate::mechanic::watch_page::{self, Watch};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/mechanic/{run}/{*file}",
            about: "The mechanic page for a game, and its routes",
            handler: mechanic,
        },
        Route {
            path: "/api/mechanic/{run}/ready",
            about: "Whether the game's page is ready; asking starts making it (the first time after a new gp it builds the game's code)",
            handler: ready,
        },
        Route {
            path: "/api/mechanic/{run}/open",
            about: "Opens a mechanic in the game's mechanic page: {mechanic?, prompt?}",
            handler: open,
        },
        Route {
            path: "/api/mechanic/{run}",
            about: "The mechanic open now in the game's mechanic page",
            handler: current,
        },
        Route {
            path: "/watch/{run}/{name}/{*file}",
            about: "The watch panel of a session: the agent's log, pins and problems, and notes to it",
            handler: watch,
        },
    ]
}

static MECHANIC: Mutex<Option<HashMap<String, Arc<Server>>>> = Mutex::new(None);
static WATCH: Mutex<Option<HashMap<String, Arc<Watch>>>> = Mutex::new(None);
/// The games whose server is being made now, with when it started, or
/// why making it failed. Making one can build the game's code (the first
/// time after a new gp, half a minute or more), so it is done on a thread
/// of its own and the page asks again; one game is made once at a time.
static MAKING: Mutex<Option<HashMap<String, Making>>> = Mutex::new(None);

#[derive(Clone)]
enum Making {
    Since(std::time::Instant),
    Failed(String),
}

/// Where a game's server is.
enum State {
    Ready(Arc<Server>),
    Making(u64),
    Failed(String),
}

/// The game's server if it is made; else starts making it (once) and
/// says how long that has run, or why it failed. `again` forgets a
/// failure and tries again.
fn state(ctx: &Ctx, game: &str, again: bool) -> Result<State, Fail> {
    if !super::game::is_name(game) {
        return Err(Fail::not_found("no such game"));
    }
    if let Some(s) = MECHANIC
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .get(game)
    {
        return Ok(State::Ready(s.clone()));
    }
    let mut making = MAKING.lock().unwrap_or_else(|e| e.into_inner());
    let making = making.get_or_insert_with(HashMap::new);
    match making.get(game) {
        Some(Making::Since(t)) => return Ok(State::Making(t.elapsed().as_secs())),
        Some(Making::Failed(why)) if !again => return Ok(State::Failed(why.clone())),
        _ => {}
    }
    // Made meanwhile by the thread that just finished.
    if let Some(s) = MECHANIC
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .get(game)
    {
        return Ok(State::Ready(s.clone()));
    }
    making.insert(game.to_owned(), Making::Since(std::time::Instant::now()));
    let (paths, games, port, name) = (
        ctx.paths.clone(),
        ctx.games.clone(),
        ctx.port,
        game.to_owned(),
    );
    let module = ctx.modules.get(game).cloned();
    std::thread::spawn(move || {
        let start = mserver::Start {
            game: &name,
            mechanic: None,
            prompt: None,
            port,
            games: Some(&games),
            module: module.as_deref(),
            // gp serve follows no build of its own for a page it mounts.
            follow: false,
        };
        let made = mserver::prepare(&paths, &start, port);
        // The server goes in before the mark comes out, so no request in
        // between starts a second one.
        let failed = match made {
            Ok((s, _)) => {
                MECHANIC
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_or_insert_with(HashMap::new)
                    .insert(name.clone(), s);
                None
            }
            Err(e) => Some(format!("{e:#}")),
        };
        let mut making = MAKING.lock().unwrap_or_else(|e| e.into_inner());
        let making = making.get_or_insert_with(HashMap::new);
        match failed {
            Some(why) => making.insert(name, Making::Failed(why)),
            None => making.remove(&name),
        };
    });
    Ok(State::Making(0))
}

/// The game's server, waited for: for the routes a command calls (`gp
/// mechanic start`), which can wait; `asked` opens that mechanic.
fn server(
    ctx: &Ctx,
    game: &str,
    asked: Option<&str>,
    prompt: Option<&str>,
) -> Result<Arc<Server>, Fail> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(900);
    let s = loop {
        match state(ctx, game, false)? {
            State::Ready(s) => break s,
            State::Failed(why) => return Err(Fail::from(anyhow::anyhow!(why))),
            State::Making(_) if std::time::Instant::now() < until => {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            State::Making(n) => {
                return Err(Fail::conflict(format!(
                    "the game is still being made ready after {n} s"
                )));
            }
        }
    };
    if asked.is_some() || prompt.is_some() {
        s.open_mechanic(asked, prompt)?;
    }
    Ok(s)
}

/// The game's server when it is made already; never starts making it.
pub(super) fn made(game: &str) -> Option<Arc<Server>> {
    MECHANIC
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()?
        .get(game)
        .cloned()
}

/// The page shown while the game is made ready, or when that failed: the
/// page's own look, a plain sentence in the status line and the game pane,
/// and, while it waits, a look every second that loads the page when the
/// game is ready.
fn waiting_page(game: &str, made: &State) -> Response {
    let esc = |t: &str| {
        t.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let (line, more, script) = match made {
        State::Failed(why) => (
            "The game couldn't be made ready.".to_owned(),
            format!(
                "<p>Tell your agent: \"The page for {game} couldn't get the game ready.\" and give it what the page says below. Then press TRY AGAIN.</p><p><a class=wait-btn href=\"./?again=1\">TRY AGAIN</a></p><pre class=muted id=wait-why>{}</pre>",
                esc(why)
            ),
            String::new(),
        ),
        _ => (
            "Getting the game ready. This takes about half a minute the first time.".to_owned(),
            "<p class=muted id=wait-time></p>".to_owned(),
            format!(
                "<script>const t0=Date.now();setInterval(async()=>{{document.getElementById('wait-time').textContent=`${{Math.round((Date.now()-t0)/1000)}} s so far. The page opens by itself when the game is ready.`;try{{const r=await fetch('/api/mechanic/{game}/ready');const v=await r.json();if(v.ready||v.failed)location.reload();}}catch(e){{}}}},1000);</script>"
            ),
        ),
    };
    let body = format!(
        "<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\"><title>Game Printer</title><link rel=stylesheet href=\"style.css\"><style>#wait{{padding:24px 16px;max-width:60ch}}#wait .wait-say{{color:var(--nes-28);font-weight:700}}.wait-btn{{font:bold 13px var(--mono);color:var(--fg);border:2px solid var(--line);padding:2px 10px;text-decoration:none}}#wait-why{{white-space:pre-wrap;font-size:12px}}</style></head><body><header><span id=logo>GAME PRINTER</span><span id=title>{game}</span><div id=next-line><span class=nx-stands id=wait-line>{line}</span></div></header><main style=\"display:block\"><section id=wait aria-live=polite><p class=wait-say>{line}</p>{more}</section></main>{script}</body></html>"
    );
    let mut r = Response::bytes("text/html; charset=utf-8", body.into_bytes());
    r.status = 200;
    r
}

/// The request as the mounted server sees it: the path under the mount.
fn under(req: &Request, file: &str) -> Request {
    let mut r = req.clone();
    r.path = format!("/{file}");
    r
}

fn mechanic(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let game = params.get("run").map(String::as_str).unwrap_or("");
    let file = params.get("file").map(String::as_str).unwrap_or("");
    if file.is_empty() {
        remember(ctx, game);
    }
    let again = file.is_empty() && req.param("again") == Some("1");
    let s = match state(ctx, game, again)? {
        State::Ready(s) => s,
        made => {
            return Ok(match file {
                "" | "index.html" => waiting_page(game, &made),
                // The waiting page's look, from the page's own files.
                "style.css" => match mserver::repo_root()
                    .and_then(|r| Ok(std::fs::read(r.join("mechanic/style.css"))?))
                {
                    Ok(b) => Response::bytes("text/css; charset=utf-8", b),
                    Err(_) => Response::error(404, "not here"),
                },
                // Anything else (a route a command or a test calls) waits
                // for the game, as before.
                _ => {
                    let s = server(ctx, game, None, None)?;
                    let res = mserver::answer(&s, &under(req, file));
                    if s.wants_reload() {
                        drop_server(game, &s);
                    }
                    res
                }
            });
        }
    };
    let res = mserver::answer(&s, &under(req, file));
    if s.wants_reload() {
        drop_server(game, &s);
    }
    Ok(res)
}

/// `GET /api/mechanic/{run}/ready`: whether the game's page is ready, for
/// the page that waits; asking starts making it.
fn ready(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let game = params.get("run").map(String::as_str).unwrap_or("");
    Ok(Response::json(&match state(ctx, game, false)? {
        State::Ready(_) => json!({"ready": true}),
        State::Making(n) => json!({"ready": false, "seconds": n}),
        State::Failed(why) => json!({"ready": false, "failed": why}),
    }))
}

/// Forgets a game's server so the next request makes it again: another
/// ROM set was chosen for it.
fn drop_server(game: &str, s: &Arc<Server>) {
    let mut all = MECHANIC.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = all.as_mut()
        && map.get(game).is_some_and(|x| Arc::ptr_eq(x, s))
    {
        map.remove(game);
    }
}

/// The last game whose page was opened, kept for gp serve's root:
/// `$GP_HOME/work/serve/last-game`.
fn last_file(ctx: &Ctx) -> std::path::PathBuf {
    ctx.paths.home.join("work").join("serve").join("last-game")
}

fn remember(ctx: &Ctx, game: &str) {
    let f = last_file(ctx);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, format!("{game}\n"));
}

/// The game whose page was opened last, if it is still a game here.
pub(super) fn last_game(ctx: &Ctx) -> Option<String> {
    let g = std::fs::read_to_string(last_file(ctx))
        .ok()?
        .trim()
        .to_owned();
    super::game::all_runs(ctx).contains(&g).then_some(g)
}

fn open(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let game = params.get("run").map(String::as_str).unwrap_or("");
    let v: Value = if req.body.is_empty() {
        json!({})
    } else {
        super::body(req)?
    };
    let (m, p) = (v["mechanic"].as_str(), v["prompt"].as_str());
    // gp mechanic start --rom chose a set (kept already): the server is
    // made again for it.
    if v["reload"] == json!(true) {
        let mut all = MECHANIC.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = all.as_mut() {
            map.remove(game);
        }
    }
    let s = server(ctx, game, m.filter(|x| !x.is_empty()), p)?;
    say(&s)
}

fn current(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let game = params.get("run").map(String::as_str).unwrap_or("");
    let s = server(ctx, game, None, None)?;
    say(&s)
}

fn say(s: &Server) -> Result<Response, Fail> {
    let dir = s.session_dir()?;
    let mechanic = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_owned();
    Ok(Response::json(
        &json!({"mechanic": mechanic, "session": dir}),
    ))
}

fn watch(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let game = params.get("run").map(String::as_str).unwrap_or("");
    let name = params.get("name").map(String::as_str).unwrap_or("");
    if !crate::mechanic::name_ok(game) || !crate::mechanic::name_ok(name) {
        return Err(Fail::not_found("no such session"));
    }
    let key = format!("{game}/{name}");
    let w = {
        let mut all = WATCH.lock().unwrap_or_else(|e| e.into_inner());
        let all = all.get_or_insert_with(HashMap::new);
        match all.get(&key) {
            Some(w) => w.clone(),
            None => {
                let w = watch_page::prepare(&ctx.paths, game, name, None, ctx.port)?;
                all.insert(key, w.clone());
                w
            }
        }
    };
    let file = params.get("file").map(String::as_str).unwrap_or("");
    Ok(watch_page::answer(&w, &under(req, file)))
}
