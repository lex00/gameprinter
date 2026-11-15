//! The first screen and the person's copies (issue #5): what they want to
//! do (port a game they own, make a theme for a game, make a new game),
//! "carry on" with each copy they have, and a copy's journey step by step
//! with whose turn each step is. The page is mechanic/start.html with
//! mechanic/journeys/ (served at `/start/`); the steps and whose turn come
//! from crate::journeys, the same answer `gp progress --next --copy` and
//! the MCP tool `next` give.
//!
//! The page's buttons run gp commands the way serve/jobs.rs's do: through
//! the agent's own `gp_run` (and `template_fork`, `build_publish`), marked
//! as the person's, so the policy, the agent's log and the running record
//! are the same. Nothing here publishes, deletes or spends: the share step
//! shows what to send, and sends nothing.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /start/` | The first screen (also `/` when no game or copy was opened last) |
//! | `GET /start/{file}` | Its files: start.html's scripts and styles, from mechanic/ |
//! | `GET /api/journeys` | The games (with a theme kit or not), the templates, every copy with where it stands, and the copy opened last |
//! | `POST /api/journeys/start` | `{journey: "theme", game, game_kind?, title, style}` or `{journey: "new", template, name}`: makes the copy (`gp theme look new` under `$GP_HOME/work/themes/`, or `template_fork` under `$GP_HOME/work/mcp/`) and its journey record |
//! | `GET /api/journeys/{kind}/{name}` | A copy: its `next` answer (steps, whose turn, facts), its buttons' last runs, and its files; opening it makes it the copy opened last |
//! | `POST /api/journeys/{kind}/{name}/set` | `{style?}` or `{step, done}`: the person's style words, or a step only they can finish marked done or not |
//! | `GET`, `POST /api/journeys/{kind}/{name}/do/{which}` | A button's last run, or starts it: a theme's `look-check`, `look-play`, `shots`; a new game's `build`, `render`, `verify`, `undo` |
//! | `GET /api/journeys/{kind}/{name}/file/{*file}` | `stage.png` (the stage by what each piece is), `shots/...` (the theme in the game), `play/...` (the new game built to play) |
//! | `GET /share/{*file}` | The page and the person's own registry together (`gp game share`'s, `$GP_HOME/work/share/`), under the public page's headers: `/share/p?g=local/NAME` plays a new game they shared |

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

use super::{Ctx, Fail, Params, Route, body, post_only};
use crate::journeys::{self as j, Kind};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/start/{*file}",
            about: "The first screen: port a game you own, make a theme, make a new game, and carry on with your copies",
            handler: page_file,
        },
        Route {
            path: "/api/journeys",
            about: "The games, the templates, your copies with where each stands, and the copy opened last",
            handler: overview,
        },
        Route {
            path: "/api/journeys/start",
            about: "Makes a copy: {journey: theme, game, title, style} or {journey: new, template, name}",
            handler: start,
        },
        Route {
            path: "/api/journeys/{kind}/{name}",
            about: "A copy's journey: its steps, whose turn, the next step, its buttons' runs and its files",
            handler: detail,
        },
        Route {
            path: "/api/journeys/{kind}/{name}/set",
            about: "The person's style words ({style}) or a step they finish ({step, done})",
            handler: set,
        },
        Route {
            path: "/api/journeys/{kind}/{name}/do/{which}",
            about: "A copy's button: GET its last run, POST starts it (look-check, look-play, shots; build, render, verify, undo)",
            handler: act,
        },
        Route {
            path: "/share/{*file}",
            about: "The page and your own registry together (gp game share): /share/p?g=local/NAME plays a new game you shared, on this computer",
            handler: shared,
        },
        Route {
            path: "/api/journeys/{kind}/{name}/file/{*file}",
            about: "A copy's pictures: stage.png, shots/..., and play/... (the new game built to play)",
            handler: file,
        },
    ]
}

// ---------------------------------------------------------------------
// The page's files.

/// The files /start/ may serve from mechanic/: its page, its scripts and
/// styles, and the page's shared look and picture card.
const FILES: &[&str] = &[
    "style.css",
    "start.html",
    "picture-card.js",
    "picture-card.css",
    "sound-card.js",
    "journeys/start.js",
    "journeys/journey.js",
    "journeys/journeys.css",
];

fn page_file(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let f = params.get("file").map(String::as_str).unwrap_or("");
    if f.is_empty() || f == "index.html" {
        return Ok(start_page(ctx));
    }
    if !FILES.contains(&f) {
        return Err(Fail::not_found("not here"));
    }
    let kind = if f.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if f.ends_with(".css") {
        "text/css; charset=utf-8"
    } else {
        "text/html; charset=utf-8"
    };
    let b = std::fs::read(ctx.repo.join("mechanic").join(f))
        .map_err(|_| Fail::not_found("not here"))?;
    Ok(Response::bytes(kind, b))
}

/// The first screen: mechanic/start.html with its port part filled in
/// here (the words of the port's first steps, or the games to carry on
/// with), so it reads the same with or without its script.
pub(super) fn start_page(ctx: &Ctx) -> Response {
    let html = std::fs::read_to_string(ctx.repo.join("mechanic/start.html")).unwrap_or_else(|_| {
        "<!doctype html><title>Game Printer</title><p>The first screen's file, mechanic/start.html, isn't here.</p><!--PORT-->".into()
    });
    let games = super::game::all_runs(ctx);
    let port = if games.is_empty() {
        super::first_screen(ctx)
    } else {
        String::new()
    };
    // The games being ported, as links, so the list is there before the
    // script runs (start.js draws it again with where each stands).
    let items: String = games
        .iter()
        .map(|g| format!("<li><a href=\"/mechanic/{g}/\">{g}</a></li>"))
        .collect();
    let mut html = html.replace("<!--PORT-->", &port).replace(
        "<ul id=\"games\" aria-label=\"The games you are porting\"></ul>",
        &format!("<ul id=\"games\" aria-label=\"The games you are porting\">{items}</ul>"),
    );
    if !games.is_empty() {
        html = html.replace("<section id=\"carry\" hidden>", "<section id=\"carry\">");
    }
    Response::bytes("text/html; charset=utf-8", html.into_bytes())
}

// ---------------------------------------------------------------------
// What there is.

fn copy_json(c: &j::Copy) -> Value {
    json!({
        "journey": c.kind.word(),
        "name": c.name,
        "dir": c.dir,
        "from": c.from,
        "touched_ms": c.touched_ms,
    })
}

fn overview(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    let home = &ctx.paths.home;
    let gs: Vec<Value> = j::games(Some(&ctx.games))
        .into_iter()
        .map(|(name, dir)| {
            let port = crate::next::gather(&ctx.paths, Some(&ctx.games), Some(&name), None)
                .ok()
                .map(|f| crate::next::answer(&f));
            json!({
                "name": name,
                "title": j::title_of(&dir),
                "kit": dir.join("themes/kit/kit.toml").is_file(),
                "touched_ms": port_touched(home, &name),
                "port": port.map(|a| json!({"stage": a.stage, "who": a.next.who, "says": a.next.says})),
            })
        })
        .collect();
    let templates: Vec<Value> = gp_mcp::fork::template_dirs(&super::jobs::config(ctx)?)
        .into_iter()
        .map(|(name, dir)| {
            let m = gp_mcp::fork::manifest(&dir);
            let get = |k: &str| {
                m.as_ref()
                    .and_then(|m| m.get("template")?.get(k)?.as_str().map(str::to_owned))
            };
            json!({
                "name": name,
                "title": get("title").unwrap_or_else(|| name.clone()),
                "open": get("tier").as_deref() == Some("open"),
                "kit": dir.join("level-kit/kit.toml").is_file(),
            })
        })
        .collect();
    let all = j::list(home, Some(&ctx.games));
    Ok(Response::json(&json!({
        "games": gs,
        "templates": templates,
        "copies": j::lines(&ctx.paths, &all),
        "last": j::last_copy(home),
        "themes_dir": j::themes_root(home),
        "copies_dir": j::copies_root(home),
    })))
}

/// When a port was last worked on: its quest log or the server's folder.
fn port_touched(home: &Path, game: &str) -> u64 {
    j::newest(&home.join("mechanics").join(game))
        .max(j::newest(&home.join("work").join("serve").join(game)))
}

// ---------------------------------------------------------------------
// Making a copy.

/// A copy's folder name from a title: lowercase letters, digits and `-`.
fn slug(title: &str) -> String {
    let mut s = String::new();
    for c in title.trim().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').chars().take(40).collect::<String>();
    if s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        s
    } else {
        format!("my-{s}").trim_end_matches('-').to_owned()
    }
}

/// The reply of a call the page made, or its failure in words. `gp_run`
/// answers with the command's exit and JSON; a named tool (`template_fork`)
/// with its own JSON, where an `error` or `refused` is its refusal.
fn reply_or_words(r: Option<gp_mcp::Reply>, gp_run: bool) -> Result<Value, Fail> {
    let Some(r) = r else {
        return Err(Fail::from(anyhow::anyhow!("that tool isn't in this build")));
    };
    if !gp_run {
        return match r.json["error"].as_str().or(r.json["refused"].as_str()) {
            Some(why) => Err(Fail::conflict(why.to_owned())),
            None => Ok(r.json),
        };
    }
    let v = super::jobs::ended(&r);
    if v["state"] == json!("done") && v["exit"].as_i64().unwrap_or(0) == 0 {
        return Ok(if v["json"].is_null() {
            r.json
        } else {
            v["json"].clone()
        });
    }
    // A named tool's refusal is its reply's error.
    let why = v["error"]
        .as_str()
        .or(r.json["error"].as_str())
        .unwrap_or("it didn't work")
        .to_owned();
    Err(Fail::conflict(why))
}

fn start(ctx: &Ctx, req: &Request, _params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let b: Value = body(req)?;
    let home = &ctx.paths.home;
    let config = super::jobs::config(ctx)?;
    let s = |k: &str| b[k].as_str().map(str::trim).unwrap_or("").to_owned();
    let page = gp_mcp::tools::Started::Page { reads_only: false };
    match s("journey").as_str() {
        "theme" => {
            let (game, title, style) = (s("game"), s("title"), s("style"));
            if title.is_empty() {
                return Err(Fail::bad_request("give your theme a name"));
            }
            let from_kind = if s("game_kind") == "new" {
                "new"
            } else {
                "game"
            };
            let game_arg = if from_kind == "new" {
                let d = j::copies_root(home).join(&game);
                if !j::is_name(&game) || !d.join(gp_model::MODEL_FILE).is_file() {
                    return Err(Fail::bad_request(format!(
                        "you have no new game called {game}"
                    )));
                }
                d.display().to_string()
            } else {
                if !j::games(Some(&ctx.games)).iter().any(|(g, _)| *g == game) {
                    return Err(Fail::bad_request(format!("there's no game {game} here")));
                }
                game.clone()
            };
            let base = slug(&title);
            let root = j::themes_root(home);
            std::fs::create_dir_all(&root)?;
            let name = (1..100)
                .map(|i| {
                    if i == 1 {
                        base.clone()
                    } else {
                        format!("{base}-{i}")
                    }
                })
                .find(|n| !root.join(n).exists())
                .ok_or_else(|| Fail::conflict("too many themes of that name"))?;
            let dir = root.join(&name);
            let args = json!({"command": "theme look new", "args": {
                "dir": dir, "game": game_arg, "name": format!("local/{name}"), "title": title}});
            reply_or_words(gp_mcp::tools::call_by(&config, "gp_run", args, page), true)?;
            j::write(
                home,
                Kind::Theme,
                &name,
                "journey.json",
                &json!({"journey": "theme", "name": name, "from": game, "from_kind": from_kind,
                        "title": title, "style": style, "made_ms": j::now_ms(), "steps_done": []}),
            )?;
            j::remember(home, Kind::Theme, &name);
            Ok(Response::json(&json!({"journey": "theme", "name": name})))
        }
        "new" => {
            let (template, name) = (s("template"), s("name"));
            if !gp_model::path::is_name(&name) {
                return Err(Fail::bad_request(
                    "a new game's name is letters, digits and _, starting with a letter (like my_hop)",
                ));
            }
            if j::copies_root(home).join(&name).exists() {
                return Err(Fail::conflict(format!(
                    "you have a game called {name} already: pick another name"
                )));
            }
            let args = json!({"template": template, "copy": name});
            let made = reply_or_words(
                gp_mcp::tools::call_by(&config, "template_fork", args, page),
                false,
            )?;
            j::write(
                home,
                Kind::New,
                &name,
                "journey.json",
                &json!({"journey": "new", "name": name, "from": template,
                        "made_ms": j::now_ms(), "steps_done": []}),
            )?;
            j::remember(home, Kind::New, &name);
            Ok(Response::json(
                &json!({"journey": "new", "name": name, "made": made}),
            ))
        }
        _ => Err(Fail::bad_request("journey is theme or new")),
    }
}

// ---------------------------------------------------------------------
// One copy.

fn the_copy(ctx: &Ctx, params: &Params) -> Result<j::Copy, Fail> {
    let kind = params
        .get("kind")
        .and_then(|k| Kind::parse(k))
        .ok_or_else(|| Fail::not_found("a copy is a theme or a new game"))?;
    let name = params.get("name").map(String::as_str).unwrap_or("");
    if !j::is_name(name) {
        return Err(Fail::not_found("no such copy"));
    }
    j::find(
        &ctx.paths.home,
        Some(&ctx.games),
        &format!("{}/{name}", kind.word()),
    )
    .map_err(|e| Fail::not_found(format!("{e:#}")))
}

/// The buttons each journey has.
fn actions(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Theme => &["look-check", "look-play", "shots"],
        Kind::New => &["build", "render", "verify", "undo", "share"],
    }
}

fn detail(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let c = the_copy(ctx, params)?;
    let home = &ctx.paths.home;
    j::remember(home, c.kind, &c.name);
    let all = j::list(home, Some(&ctx.games));
    let mut a = j::answer(&j::gather(&ctx.paths, &all, &c), &j::around(home, &c));
    a.copies = j::lines(&ctx.paths, &all);
    let jobs: serde_json::Map<String, Value> = actions(c.kind)
        .iter()
        .map(|w| (w.to_string(), job(home, &c, w)))
        .collect();
    let rd = j::record_dir(home, c.kind, &c.name);
    let base = format!("/api/journeys/{}/{}/file", c.kind.word(), c.name);
    let mut files = json!({});
    match c.kind {
        Kind::Theme => {
            let mut shots: Vec<String> = walk_png(&rd.join("shots"))
                .into_iter()
                .map(|p| format!("{base}/shots/{p}"))
                .collect();
            shots.sort();
            files["shots"] = json!(shots);
            files["check"] = j::read(home, c.kind, &c.name, "check.json");
            files["play"] = j::read(home, c.kind, &c.name, "play.json");
        }
        Kind::New => {
            let png = rd.join("stage.png");
            if png.is_file() {
                files["stage"] = json!(format!("{base}/stage.png?t={}", mtime(&png)));
                files["stage_current"] = json!(mtime(&png) >= j::newest(&c.dir));
            }
            if play_dir(home, &c.name).join("index.html").is_file() {
                files["play"] = json!(format!("{base}/play/index.html"));
            }
            files["verify"] = j::read(home, c.kind, &c.name, "verify.json");
        }
    }
    Ok(Response::json(&json!({
        "copy": copy_json(&c),
        "answer": crate::next::json_of(&a),
        "jobs": jobs,
        "files": files,
    })))
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

/// The PNGs under a folder, by their path in it.
fn walk_png(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            out.extend(walk_png(&p).into_iter().map(|x| format!("{n}/{x}")));
        } else if n.ends_with(".png") {
            out.push(n);
        }
    }
    out
}

fn play_dir(home: &Path, name: &str) -> PathBuf {
    j::copies_root(home)
        .join(".publish")
        .join(name)
        .join("web-preview")
}

fn set(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let c = the_copy(ctx, params)?;
    let b: Value = body(req)?;
    let home = &ctx.paths.home;
    let mut r = c.record.clone();
    if !r.is_object() {
        r = json!({"journey": c.kind.word(), "name": c.name, "made_ms": j::now_ms(), "steps_done": []});
        if let Some(f) = &c.from {
            r["from"] = json!(f.name);
            r["from_kind"] = json!(f.kind);
        }
    }
    if let Some(style) = b["style"].as_str() {
        let style = style.trim();
        if style.len() > 300 {
            return Err(Fail::bad_request("say the style in a few words"));
        }
        r["style"] = json!(style);
    }
    if let Some(step) = b["step"].as_str() {
        let ok = matches!(
            (c.kind, step),
            (Kind::Theme, "pictures") | (Kind::New, "stage" | "look")
        );
        if !ok {
            return Err(Fail::bad_request(format!(
                "{step} isn't a step you finish yourself"
            )));
        }
        let mut done: BTreeSet<String> = r["steps_done"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect();
        if b["done"] == json!(false) {
            done.remove(step);
        } else {
            done.insert(step.to_owned());
        }
        r["steps_done"] = json!(done);
    }
    j::write(home, c.kind, &c.name, "journey.json", &r)?;
    Ok(Response::json(&r))
}

// ---------------------------------------------------------------------
// The buttons.

/// The buttons running in this server now, by (kind/name, which).
static BUSY: Mutex<BTreeSet<(String, String)>> = Mutex::new(BTreeSet::new());

fn job(home: &Path, c: &j::Copy, which: &str) -> Value {
    let v = j::read(home, c.kind, &c.name, &format!("do-{which}.json"));
    if v.is_null() {
        return json!({"state": "never"});
    }
    let busy = BUSY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&(format!("{}/{}", c.kind.word(), c.name), which.to_owned()));
    if v["state"] == json!("running") && !busy {
        let mut v = v;
        v["state"] = json!("failed");
        v["error"] = json!("the page's server stopped while it ran");
        return v;
    }
    v
}

/// The call a button makes: the tool, its arguments, and whether it only
/// reads.
fn call_of(home: &Path, c: &j::Copy, which: &str) -> Result<(&'static str, Value, bool), Fail> {
    let dir = c.dir.display().to_string();
    let game = || {
        c.from.as_ref().map(j::Origin::game_arg).ok_or_else(|| {
            Fail::conflict("this theme doesn't say which game it is for yet: tell your agent")
        })
    };
    let rd = j::record_dir(home, c.kind, &c.name);
    let run = |command: &str, args: Value, timeout: u64| json!({"command": command, "args": args, "timeout_s": timeout});
    Ok(match (c.kind, which) {
        (Kind::Theme, "look-check") => (
            "gp_run",
            run(
                "theme look check",
                json!({"dir": dir, "game": game()?}),
                600,
            ),
            true,
        ),
        (Kind::Theme, "look-play") => (
            "gp_run",
            run(
                "theme look play",
                json!({"dir": dir, "game": game()?, "background": true, "minutes": 60}),
                1200,
            ),
            true,
        ),
        (Kind::Theme, "shots") => (
            "gp_run",
            run(
                "theme look play",
                json!({"dir": dir, "game": game()?, "shots": rd.join("shots")}),
                1800,
            ),
            true,
        ),
        (Kind::New, "build") => ("build_publish", json!({"copy": c.name}), false),
        (Kind::New, "render") => (
            "gp_run",
            run(
                "level render",
                json!({"game": c.name, "out": rd.join("stage.png")}),
                300,
            ),
            true,
        ),
        (Kind::New, "verify") => (
            "gp_run",
            run("level verify", json!({"game": c.name}), 900),
            true,
        ),
        (Kind::New, "share") => (
            "gp_run",
            run("game share", json!({"dir": dir}), 1800),
            false,
        ),
        (Kind::New, "undo") => (
            "gp_run",
            run("level undo", json!({"game": c.name}), 300),
            false,
        ),
        _ => {
            return Err(Fail::not_found(format!(
                "no button {which}: this copy has {}",
                actions(c.kind).join(", ")
            )));
        }
    })
}

fn act(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let c = the_copy(ctx, params)?;
    let which = params.get("which").cloned().unwrap_or_default();
    let home = ctx.paths.home.clone();
    if req.method != "POST" {
        return Ok(Response::json(&job(&home, &c, &which)));
    }
    let (tool, args, reads_only) = call_of(&home, &c, &which)?;
    let key = (format!("{}/{}", c.kind.word(), c.name), which.clone());
    if !BUSY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.clone())
    {
        return Err(Fail::conflict(format!("{which} is running already")));
    }
    let config = match super::jobs::config(ctx) {
        Ok(c) => c,
        Err(e) => {
            BUSY.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
            return Err(e);
        }
    };
    let file = format!("do-{which}.json");
    let base = json!({"action": which, "tool": tool, "started_ms": j::now_ms()});
    let mut running = base.clone();
    running["state"] = json!("running");
    j::write(&home, c.kind, &c.name, &file, &running)?;
    let (kind, name) = (c.kind, c.name.clone());
    std::thread::spawn(move || {
        let started = gp_mcp::tools::Started::Page { reads_only };
        let mut v = match gp_mcp::tools::call_by(&config, tool, args, started) {
            Some(reply) if tool == "gp_run" => super::jobs::ended(&reply),
            // A named tool answers with its own JSON: an `error` is its
            // refusal.
            Some(reply) => match reply.json["error"]
                .as_str()
                .or(reply.json["refused"].as_str())
            {
                Some(e) => json!({"state": "failed", "error": e}),
                None if reply.json["ok"] == json!(false) => json!({"state": "failed", "error":
                    reply.json["problems"].to_string(), "json": reply.json}),
                None => json!({"state": "done", "exit": 0, "json": reply.json}),
            },
            None => json!({"state": "failed", "error": "that tool isn't in this build"}),
        };
        for (k, x) in base.as_object().into_iter().flatten() {
            v[k] = x.clone();
        }
        v["ended_ms"] = json!(j::now_ms());
        let _ = j::write(&home, kind, &name, &file, &v);
        BUSY.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
    });
    Ok(Response::json(&running))
}

fn file(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let c = the_copy(ctx, params)?;
    let f = params.get("file").map(String::as_str).unwrap_or("");
    if f.split('/')
        .any(|p| p.is_empty() || p == ".." || p.starts_with('.'))
    {
        return Err(Fail::not_found("not here"));
    }
    let home = &ctx.paths.home;
    let path = match (c.kind, f.split_once('/')) {
        (Kind::New, Some(("play", rest))) => play_dir(home, &c.name).join(rest),
        (Kind::Theme, Some(("shots", _))) | (Kind::New, None) if f.ends_with(".png") => {
            j::record_dir(home, c.kind, &c.name).join(f)
        }
        _ => return Err(Fail::not_found("not here")),
    };
    let kind = match Path::new(f).extension().and_then(|x| x.to_str()) {
        Some("png") => "image/png",
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    };
    let b = std::fs::read(&path).map_err(|_| Fail::not_found("not here"))?;
    // The new game built to play is a game's frame: served as the public
    // page serves its own.
    if f.starts_with("play/") {
        return Ok(Response::game_file(kind, b));
    }
    Ok(Response::bytes(kind, b))
}

/// The page and the person's registry, served together as `gp registry
/// serve` serves them, at `/share/`: the page's files first, then the
/// registry's, each under the public page's own headers (a game's frame is
/// among them). `/share/` and `/share/p` are the page.
fn shared(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let f = params.get("file").map(String::as_str).unwrap_or("");
    let f = if f.is_empty() || f == "p" {
        "index.html"
    } else {
        f
    };
    if f.split('/')
        .any(|p| p.is_empty() || p == ".." || p.starts_with('.'))
        || f.contains('\\')
    {
        return Err(Fail::not_found("not here"));
    }
    let share = crate::game_share::share_dir(&ctx.paths.home);
    let Some(path) = [share.join("page"), share.join("registry")]
        .iter()
        .map(|d| d.join(f))
        .find(|p| p.is_file())
    else {
        return Err(Fail::not_found(
            "not here: nothing is shared yet, or not this file (gp game share shares a new game)",
        ));
    };
    let kind = match Path::new(f).extension().and_then(|x| x.to_str()) {
        Some("png") => "image/png",
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("toml" | "txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    let b = std::fs::read(&path).map_err(|_| Fail::not_found("not here"))?;
    Ok(Response::game_file(kind, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_makes_a_folder_name() {
        assert_eq!(slug("Night Time, Neon!"), "night-time-neon");
        assert_eq!(slug("  2 Moons "), "my-2-moons");
        assert_eq!(slug("..."), "my");
    }
}
