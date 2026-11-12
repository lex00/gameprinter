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

/// The game's mechanic server, made once; `asked` opens that mechanic.
fn server(
    ctx: &Ctx,
    game: &str,
    asked: Option<&str>,
    prompt: Option<&str>,
) -> Result<Arc<Server>, Fail> {
    if !super::game::is_name(game) {
        return Err(Fail::not_found("no such game"));
    }
    let mut all = MECHANIC.lock().unwrap_or_else(|e| e.into_inner());
    let all = all.get_or_insert_with(HashMap::new);
    if let Some(s) = all.get(game) {
        if asked.is_some() || prompt.is_some() {
            s.open_mechanic(asked, prompt)?;
        }
        return Ok(s.clone());
    }
    let start = mserver::Start {
        game,
        mechanic: asked,
        prompt,
        port: ctx.port,
        games: Some(&ctx.games),
        module: ctx.modules.get(game).map(std::path::PathBuf::as_path),
        // gp serve follows no build of its own for a page it mounts.
        follow: false,
    };
    let (s, _) = mserver::prepare(&ctx.paths, &start, ctx.port)?;
    all.insert(game.to_owned(), s.clone());
    Ok(s)
}

/// The request as the mounted server sees it: the path under the mount.
fn under(req: &Request, file: &str) -> Request {
    let mut r = req.clone();
    r.path = format!("/{file}");
    r
}

fn mechanic(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let game = params.get("run").map(String::as_str).unwrap_or("");
    let s = server(ctx, game, None, None)?;
    let file = params.get("file").map(String::as_str).unwrap_or("");
    if file.is_empty() {
        remember(ctx, game);
    }
    let res = mserver::answer(&s, &under(req, file));
    if s.wants_reload() {
        drop_server(game, &s);
    }
    Ok(res)
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
