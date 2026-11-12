//! A run and its game (game-printer's serve/game.rs and the run list of
//! serve/capture.rs). In gameprinter a run is a game of the games
//! checkout: `games/GAME`, named by its folder. What the server keeps for
//! it is in `$GP_HOME/work/serve/GAME/`.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture` | The runs: every game in the games checkout |
//! | `GET /api/capture/{run}/game` | The game in its own words: the model's title, entities, levels, counters, rule states and input actions; its recordings; whether the ROM and the rebuilt game are there; and the screen, frame rate and buttons of its platform |
//! | `GET /api/capture/{run}/rom` | The game's scored ROM, for the page's player; never written anywhere |
//! | `GET /api/capture/{run}/preview/{file}` | A file of the rebuilt game's interpreter preview, built as the mechanic page builds it |
//! | `GET /api/capture/{run}/events` | The run's events: none, since a game has no capture run's event log |
//!
//! Nothing here knows a game: the words are the model's own names.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};
use crate::mechanic::review::Rebuild;
use crate::roms::Platform;

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/capture",
            about: "The runs: every game in the games checkout",
            handler: runs,
        },
        Route {
            path: "/api/capture/{run}/game",
            about: "A run's game in its own words, its recordings, whether the ROM and the rebuilt game are there, and its platform's screen, frame rate and buttons",
            handler: game,
        },
        Route {
            path: "/api/capture/{run}/rom",
            about: "The game's scored ROM, for the page's player; never written anywhere",
            handler: rom,
        },
        Route {
            path: "/api/capture/{run}/preview/{*file}",
            about: "A file of the rebuilt game's interpreter preview (built on first ask, as the mechanic page builds it)",
            handler: preview,
        },
        Route {
            path: "/api/capture/{run}/events",
            about: "The run's events since `after`: the background work's, as it happens (super::background::emit)",
            handler: events,
        },
        Route {
            path: "/api/nes/colors",
            about: "The NES's 64 colors as hex, for the views that draw tiles",
            handler: colors,
        },
    ]
}

/// A run: a game of the games checkout and the server's folder for it.
pub(crate) struct Run {
    pub name: String,
    /// `games/GAME`.
    pub game: PathBuf,
    /// `$GP_HOME/work/serve/GAME`.
    pub dir: PathBuf,
}

impl Run {
    pub fn platform(&self) -> Platform {
        crate::package::Package::load(&self.game)
            .and_then(|p| p.platform())
            .unwrap_or(Platform::Nes)
    }

    /// The run's working copy of the game's folder, which the background
    /// work's refine rounds write: `$GP_HOME/work/serve/GAME/model/`.
    pub fn copy_dir(&self) -> PathBuf {
        self.dir.join("model")
    }

    /// The model the run plays and scores: the working copy once there is
    /// one, else the game's own folder.
    pub fn model_dir(&self) -> PathBuf {
        let c = self.copy_dir();
        if c.join(gp_model::MODEL_FILE).is_file() {
            c
        } else {
            self.game.clone()
        }
    }

    /// Makes the working copy from the game's folder if there is none.
    pub fn ensure_copy(&self) -> std::io::Result<PathBuf> {
        let c = self.copy_dir();
        if !c.join(gp_model::MODEL_FILE).is_file() {
            let tmp = self.dir.join(format!("model.tmp{}", std::process::id()));
            copy_tree(&self.game, &tmp)?;
            fs::rename(&tmp, &c)?;
        }
        Ok(c)
    }

    /// The model, its module and its mapping, for the rebuilt game.
    pub fn rebuild(&self, ctx: &Ctx) -> Rebuild {
        Rebuild::new(
            &ctx.paths,
            &self.model_dir(),
            &self.name,
            ctx.modules.get(&self.name).map(PathBuf::as_path),
        )
    }

    pub fn has_model(&self) -> bool {
        self.game.join(gp_model::MODEL_FILE).is_file()
    }
}

/// A name a route takes: no path separators, not hidden.
pub(crate) fn is_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0'])
}

/// The run a route's `{run}` names.
pub(crate) fn run_dir(ctx: &Ctx, params: &Params) -> Result<Run, Fail> {
    run_named(ctx, params.get("run").map(String::as_str).unwrap_or(""))
}

pub(crate) fn run_named(ctx: &Ctx, name: &str) -> Result<Run, Fail> {
    if !is_name(name) {
        return Err(Fail::not_found("no such run"));
    }
    let game = ctx.games.join(name);
    if !game.join("package.toml").is_file() {
        return Err(Fail::not_found(format!(
            "no run {name}: {} has no game {name}",
            ctx.games.display()
        )));
    }
    Ok(Run {
        name: name.to_owned(),
        game,
        dir: ctx.paths.home.join("work").join("serve").join(name),
    })
}

/// Every game in the checkout, by name.
pub(crate) fn all_runs(ctx: &Ctx) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(&ctx.games)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("package.toml").is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|n| is_name(n))
        .collect();
    out.sort();
    out
}

fn runs(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    let list: Vec<Value> = all_runs(ctx)
        .iter()
        .filter_map(|n| run_named(ctx, n).ok())
        .map(|r| {
            json!({
                "run": r.name,
                "model": r.game,
                "round": 0,
                "events": 0,
                "state": "idle",
                "watched": false,
                "pending_inputs": 0,
                "started_ms": 0,
                "preview": r.has_model() && r.platform() == Platform::Nes,
                "pins": [],
            })
        })
        .collect();
    Ok(Response::json(&json!({"runs": list})))
}

fn s(v: Option<&toml::Value>) -> Value {
    v.and_then(|x| x.as_str())
        .map(|x| json!(x))
        .unwrap_or(Value::Null)
}

fn to_json(v: Option<&toml::Value>) -> Value {
    v.and_then(|x| serde_json::to_value(x).ok())
        .unwrap_or(Value::Null)
}

/// The model's words, read from its file without the schema, so a model
/// of any version (or one that doesn't check) still gets its names.
pub(crate) fn vocab(model: &toml::Value) -> Value {
    let table = |k: &str| model.get(k).and_then(|v| v.as_table());
    let entities: Vec<Value> = table("entities")
        .map(|t| {
            t.iter()
                .map(|(name, e)| {
                    json!({"name": name, "role": s(e.get("role")), "description": s(e.get("description"))})
                })
                .collect()
        })
        .unwrap_or_default();
    let levels: Vec<Value> = table("levels")
        .map(|t| {
            t.iter()
                .map(|(name, l)| {
                    json!({"name": name, "description": s(l.get("description")),
                           "size": to_json(l.get("size")), "tile_size": to_json(l.get("tile_size"))})
                })
                .collect()
        })
        .unwrap_or_default();
    let counters: Vec<Value> = table("counters")
        .map(|t| t.keys().map(|k| json!(k)).collect())
        .unwrap_or_default();
    let mut rules = Map::new();
    for (name, r) in table("rules").into_iter().flatten() {
        let states: Vec<Value> = r
            .get("states")
            .and_then(|v| v.as_table())
            .map(|t| t.keys().map(|k| json!(k)).collect())
            .unwrap_or_default();
        rules.insert(name.clone(), json!(states));
    }
    let m = model.get("model");
    json!({
        "name": s(m.and_then(|m| m.get("name"))),
        "title": s(m.and_then(|m| m.get("title"))),
        "view": s(m.and_then(|m| m.get("view"))),
        "entities": entities,
        "levels": levels,
        "counters": counters,
        "rules": rules,
        "actions": to_json(model.get("input").and_then(|i| i.get("actions"))),
    })
}

/// The screen, the frame rate and the buttons of the run's platform, as the
/// player module says them (`gp_play_info`).
pub(crate) fn screen(platform: Platform) -> Value {
    match platform {
        Platform::Nes => serde_json::from_str(&gp_play::info()).unwrap_or(Value::Null),
        Platform::Arcade => arcade_screen(None),
    }
}

/// An arcade game's screen, frame rate and buttons, as gp-arcade-play's
/// `gp_play_info` gives them (its `info`): gp_arcade's screen, 30 frames a
/// second, and the cabinet's buttons in the arcade input log's columns
/// (`CSRLDUT`).
pub(crate) fn arcade_screen(system: Option<&str>) -> Value {
    json!({
        "platform": "arcade",
        "system": system,
        "width": gp_arcade::video::WIDTH,
        "height": gp_arcade::video::HEIGHT,
        "fps": [30, 1],
        "columns": "CSRLDUT",
        "buttons": ["coin", "start1", "right", "left", "down", "up", "button1"],
        "sound": true,
        "channels": 2,
    })
}

/// The run's screen: the platform's, with an arcade game's set named.
pub(crate) fn run_screen(run: &Run) -> Value {
    match run.platform() {
        Platform::Nes => screen(Platform::Nes),
        Platform::Arcade => {
            let system = crate::package::Package::load(&run.game).ok().and_then(|p| {
                let e = p
                    .entries
                    .iter()
                    .find(|e| e.role == "scored")
                    .or(p.entries.first())?;
                e.system.clone()
            });
            arcade_screen(system.as_deref())
        }
    }
}

fn game(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    let model: toml::Value = fs::read_to_string(run.game.join(gp_model::MODEL_FILE))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or(toml::Value::Table(Default::default()));
    let platform = run.platform();
    let preview = run.has_model() && platform == Platform::Nes;
    if preview && !crate::mechanic::live::is_built(&ctx.paths, &run.name) {
        // The first ask builds it, in the background.
        if let Ok(rom) = rom_of(ctx, &run) {
            let _ = crate::mechanic::live::build(
                &ctx.paths,
                &ctx.repo,
                &run.name,
                &run.rebuild(ctx),
                &rom,
            );
        }
    }
    Ok(Response::json(&json!({
        "run": run.name,
        "kind": "capture",
        "model": run.game,
        "template": Value::Null,
        "vocab": vocab(&model),
        "mechanics": {"mechanics": [], "paths": []},
        "recordings": [{"id": "corpus", "name": "corpus", "frames": Value::Null, "input": "corpus"}],
        "rom": rom_of(ctx, &run).is_ok(),
        "preview": preview,
        "platform": platform.name(),
        "screen": run_screen(&run),
    })))
}

/// The ROMs read so far, by game: reading one finds it by hash in the ROM
/// folder, which takes a moment.
static ROMS: Mutex<Option<HashMap<PathBuf, Arc<Vec<u8>>>>> = Mutex::new(None);

/// The game's scored ROM: the one its package.toml names, found by hash in
/// $GP_ROM_DIR.
pub(crate) fn rom_of(ctx: &Ctx, run: &Run) -> Result<Arc<Vec<u8>>, Fail> {
    if let Some(r) = ROMS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .get(&run.game)
    {
        return Ok(r.clone());
    }
    let bytes = crate::clip_score::rom_of(&ctx.paths, &run.game, None)
        .map_err(|e| Fail::not_found(format!("{e:#}")))?;
    let bytes = Arc::new(bytes);
    ROMS.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(run.game.clone(), bytes.clone());
    Ok(bytes)
}

fn rom(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    Ok(Response::bytes(
        "application/octet-stream",
        rom_of(ctx, &run)?.to_vec(),
    ))
}

/// The page shown in the rebuilt game's frame while its preview builds: it
/// asks again every few seconds.
const WAITING: &str = "<!doctype html><meta charset=utf-8><body style=\"background:#000;color:#9aa;font:13px system-ui;margin:12px\">Building the rebuilt game (the model's interpreter preview); it shows here when it's done.<script>setTimeout(()=>location.reload(),3000)</script>";

fn preview(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    if run.platform() != Platform::Nes {
        return Err(Fail::conflict(
            "the rebuilt game is the game's model, and an arcade game has none yet",
        ));
    }
    let file = params.get("file").map(String::as_str).unwrap_or("");
    let built = crate::mechanic::live::is_built(&ctx.paths, &run.name);
    if !built && (file == "index.html" || file.is_empty()) {
        if let Ok(rom) = rom_of(ctx, &run) {
            let _ = crate::mechanic::live::build(
                &ctx.paths,
                &ctx.repo,
                &run.name,
                &run.rebuild(ctx),
                &rom,
            );
        }
        return Ok(Response::bytes(
            "text/html; charset=utf-8",
            WAITING.as_bytes().to_vec(),
        ));
    }
    let file = if file.is_empty() { "index.html" } else { file };
    Ok(crate::mechanic::live::file(
        &ctx.paths, &ctx.repo, &run.name, file,
    )?)
}

fn events(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    let after = _req
        .param("after")
        .and_then(|a| a.parse().ok())
        .unwrap_or(0);
    Ok(Response::json(
        &json!({"run": run.name, "events": super::background::events_after(&run.name, after)}),
    ))
}

fn colors(_ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    let colors: Vec<String> = gp_nes::video::colors()
        .iter()
        .map(|[r, g, b]| format!("{r:02x}{g:02x}{b:02x}"))
        .collect();
    Ok(Response::json(&json!({"colors": colors})))
}

/// Reads a JSON file, or null.
pub(crate) fn read_json(file: &Path) -> Value {
    fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

/// Writes a JSON file through a temporary one beside it.
pub(crate) fn write_json(file: &Path, v: &Value) -> std::io::Result<()> {
    if let Some(d) = file.parent() {
        fs::create_dir_all(d)?;
    }
    let tmp = file.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default())?;
    fs::rename(&tmp, file)
}

/// Copies a folder, leaving out build output and git's.
pub(crate) fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &to.join(&name))?;
        } else {
            fs::copy(e.path(), to.join(&name))?;
        }
    }
    Ok(())
}
