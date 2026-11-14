//! The game view's lockstep panel (docs/inspector.md, "The game view"):
//! the original and the model on one recording's input, measured apart on
//! every mapped path, as `gp interp compare --json` gives it under
//! `lockstep`.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture/{run}/lockstep?recording=NAME&mode=resync\|free&from=FRAME` | The lockstep of the run's model with its ROM on the recording's input, in resync mode (the default) or free run, from the mapping's start or the original's frame `from`. Cached in the run's `lockstep/` folder, keyed by everything it reads |
//! | `GET /api/capture/{run}/moments?recording=NAME` | The game's own moments in the recording (`gp interp moments --json`): each jump, landing, death, life lost, counter change and level change of the original, with its frames, what it did, the mechanic that owns it, the stretch a focus on it covers, and what the model does there on the same presses from the original's state half a second before. Cached beside the lockstep |
//! | `GET /api/capture/{run}/start-state?recording=NAME&frame=N` | The original's state at frame `N` translated into a model state, as a lockstep from `N` starts: `original_frame` (the first frame the model stands for at or after `N`), the model's `frame`, the canonical `state` in base64 for the preview's `load-state`, and the translation's `notes`. Cached beside the lockstep |
//! | `GET /api/capture/{run}/inputs/{name}` | A recording's input log from power-on, as text |
//!
//! The frames are the original's, so the page lines them up with the
//! view's slider directly.
//!
//! Ported from game-printer's serve/lockstep.rs and the inputs route of
//! serve/game.rs (wave 8). What gameprinter's layout changed: a run is a
//! game, and its recordings ([`recordings`]) are its corpus clips (each
//! clip's start and presses from power-on), the clips played in free play,
//! and the store's recordings of the game's ROM, where game-printer's were
//! a capture run's baselines and sessions. The model is the run's working
//! copy once the background work made one, opened with its module as the
//! mechanic page's rebuild opens it. The moments are kept by recording in
//! `$GP_HOME/work/serve/GAME/moments/`, where gp-mcp's `moments` tool reads
//! them for an agent working on the game.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::game::{Run, rom_of, run_dir};
use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/capture/{run}/lockstep",
            about: "The original and the model in lockstep on one of the run's recordings: every mapped path's distance each frame, the splits and the mechanics they belong to (?recording=NAME&mode=resync|free&from=FRAME)",
            handler: lockstep,
        },
        Route {
            path: "/api/capture/{run}/moments",
            about: "The game's own moments in one of the run's recordings (jumps, landings, deaths, lives lost, counter changes, level changes) and what the model does at each (?recording=NAME)",
            handler: moments,
        },
        Route {
            path: "/api/capture/{run}/start-state",
            about: "The original's state at a frame of one of the run's recordings as a model state, for the rebuilt game to start from (?recording=NAME&frame=N)",
            handler: start_state,
        },
        Route {
            path: "/api/capture/{run}/inputs/{name}",
            about: "A recording's input log from power-on: a corpus clip's, a free play clip's or a store recording's of the game's ROM",
            handler: inputs,
        },
    ]
}

/// The run's recordings for the game view, by name: the corpus clips, the
/// free play clips and the store's recordings of the game's ROM. Each has
/// an input log ([`input_of`]).
pub(crate) fn recordings(ctx: &Ctx, run: &Run) -> Vec<Value> {
    let mut out = Vec::new();
    for c in super::generated::made(run) {
        out.push(json!({"id": c.id, "name": c.id, "frames": c.frames, "input": c.id}));
    }
    for c in super::freeplay::clips_for_run(&ctx.paths.home, &run.name) {
        out.push(json!({"id": c.id, "name": c.id, "frames": c.frames, "input": c.id}));
    }
    for (id, name, frames) in store_recordings(ctx, run) {
        out.push(json!({"id": id, "name": name, "frames": frames, "input": name}));
    }
    out
}

/// The store's recordings of the run's ROM entries: ID, name and frames.
fn store_recordings(ctx: &Ctx, run: &Run) -> Vec<(String, String, u64)> {
    let Ok(store) = gp_evidence::Store::open_existing(&ctx.paths.store) else {
        return Vec::new();
    };
    let shas: Vec<String> = fs::read_to_string(run.game.join("roms.toml"))
        .ok()
        .and_then(|t| gp_evidence::registry::Registry::from_toml(&t, &run.game).ok())
        .map(|r| r.sources.into_iter().map(|k| k.sha256).collect())
        .unwrap_or_default();
    let Ok(listing) = gp_evidence::recordings::list_checked(&store) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String, u64)> = Vec::new();
    for e in listing.entries.iter().rev() {
        let id = e.id.to_hex();
        if out.iter().any(|(i, n, _)| *i == id || *n == e.name) {
            continue;
        }
        let Ok(m) = gp_evidence::recordings::load_manifest(&store, &e.id) else {
            continue;
        };
        let sha = match &m.source {
            gp_evidence::manifest::SourceRef::NesRom { sha256, .. } => sha256.as_str(),
            gp_evidence::manifest::SourceRef::Romset { identity, .. } => identity.as_str(),
            gp_evidence::manifest::SourceRef::Model { .. } => continue,
        };
        if shas.iter().any(|s| s == sha) && super::game::is_name(&e.name) {
            out.push((id, e.name.clone(), m.frames));
        }
    }
    out.reverse();
    out
}

/// A recording's input log from power-on, by its name in [`recordings`]:
/// a corpus clip, a free play clip, or a store recording of the game's ROM
/// by name or ID.
pub(crate) fn input_of(ctx: &Ctx, run: &Run, name: &str) -> Result<Vec<u8>, Fail> {
    if !super::game::is_name(name) {
        return Err(Fail::not_found("no such input"));
    }
    if let Ok((log, _, _, _)) = super::scenarios::clip_input(ctx, run, name) {
        return Ok(log.to_text().into_bytes());
    }
    if let Some((id, _, _)) = store_recordings(ctx, run)
        .into_iter()
        .find(|(id, n, _)| n == name || id.starts_with(name))
    {
        let opened = super::recordings::open(ctx, &id)?;
        let bytes = opened
            .store
            .get(&opened.manifest.input_log)
            .map_err(anyhow::Error::from)?;
        return Ok(bytes);
    }
    Err(Fail::not_found(format!(
        "no input log {name} in the run: not a clip of the game's tests, a free play clip or a store recording of the game's ROM"
    )))
}

fn inputs(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    let name = params.get("name").map(String::as_str).unwrap_or("");
    let bytes = input_of(ctx, &run, name)?;
    Ok(Response::bytes("text/plain; charset=utf-8", bytes))
}

/// What a lockstep or a start state reads: the model and its mapping, the
/// recording's input and the ROM, and the cache key over all of them.
struct Inputs {
    run: Run,
    opened: std::sync::Arc<(crate::hosted::Source, gp_interp::mapping::Mapping)>,
    /// The model's lifted pictures, for the screen measure.
    assets: Option<PathBuf>,
    mdir: PathBuf,
    log: Vec<u8>,
    rom: std::sync::Arc<Vec<u8>>,
    key: Vec<u8>,
}

impl Inputs {
    fn read(ctx: &Ctx, req: &Request, params: &Params) -> Result<(Self, String), Fail> {
        let run = run_dir(ctx, params)?;
        let name = req
            .param("recording")
            .ok_or_else(|| Fail::bad_request("name a recording with ?recording="))?
            .to_owned();
        let mdir = run.model_dir();
        let mapping_text = fs::read_to_string(mdir.join(gp_interp::mapping::MAPPING_FILE))
            .map_err(|_| Fail::not_found("the model has no state mapping (mapping.toml), so the two games can't be measured apart"))?;
        let log = input_of(ctx, &run, &name)?;
        let rom = rom_of(ctx, &run)?;
        let mut rb = run.rebuild(ctx);
        rb.game = mdir.clone();
        let opened = crate::mechanic::live::opened(&ctx.paths, &rb, &rom)?;
        let assets = rb.assets_for(&ctx.paths, &rom).ok();
        let contract = fs::read(mdir.join(gp_contract::CONTRACT_FILE)).unwrap_or_default();
        let mut key = Vec::new();
        for part in [
            opened.0.bundle_bytes(),
            mapping_text.as_bytes(),
            &contract,
            &log,
            &rom[..],
            env!("CARGO_PKG_VERSION").as_bytes(),
            opened.0.hooks.as_bytes(),
        ] {
            key.extend_from_slice(&(part.len() as u64).to_le_bytes());
            key.extend_from_slice(part);
        }
        Ok((
            Self {
                run,
                opened,
                assets,
                mdir,
                log,
                rom,
                key,
            },
            name,
        ))
    }

    /// The cache file for this and `extra`, in the run's `lockstep/` folder.
    fn cache(&self, extra: &[&[u8]]) -> PathBuf {
        let mut key = self.key.clone();
        for part in extra {
            key.extend_from_slice(&(part.len() as u64).to_le_bytes());
            key.extend_from_slice(part);
        }
        let key = gp_evidence::Hash::of(&key).to_string();
        self.run
            .dir
            .join("lockstep")
            .join(format!("{}.json", &key[key.len() - 16..]))
    }

    fn frames(&self) -> Result<Vec<gp_runtime::InputFrame>, Fail> {
        let text = std::str::from_utf8(&self.log)
            .map_err(|_| Fail::bad_request("the input log isn't text"))?;
        let input = gp_evidence::InputLog::parse(text).map_err(anyhow::Error::from)?;
        Ok(crate::run::frames(&input))
    }
}

fn cached(cache: &Path) -> Option<Value> {
    fs::read(cache)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
}

fn keep(cache: &Path, j: &Value) {
    if let Some(parent) = cache.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // A cache that can't be written is only slower next time.
    let _ = fs::write(cache, serde_json::to_vec(j).unwrap_or_default());
}

/// Where gp-mcp's `moments` tool reads a recording's moments: beside the
/// game's own cache, `$GP_HOME/work/serve/GAME/moments/` (no capture run).
fn moments_file(ctx: &Ctx, run: &Run, name: &str) -> Option<PathBuf> {
    Some(gp_mcp::moments::kept_file(&ctx.paths.home, &run.name, name))
}

fn moments(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let (inp, name) = Inputs::read(ctx, req, params)?;
    let cache = inp.cache(&[b"moments"]);
    let by_name = moments_file(ctx, &inp.run, &name);
    if let Some(v) = cached(&cache) {
        if let Some(f) = &by_name {
            keep(f, &v);
        }
        return Ok(Response::json(&v));
    }
    let (source, mapping) = (&inp.opened.0, &inp.opened.1);
    let (mut j, _) = crate::moments::report(source, mapping, &inp.rom, &inp.frames()?, None)?;
    j["recording"] = Value::from(name.as_str());
    keep(&cache, &j);
    // And by recording, for the agent's moments tool (gp mcp), which
    // reads it rather than finding them again.
    if let Some(f) = &by_name {
        keep(f, &j);
    }
    Ok(Response::json(&j))
}

fn start_state(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    use base64::Engine as _;
    let frame = req
        .param("frame")
        .unwrap_or("1")
        .parse::<u64>()
        .map_err(|e| Fail::bad_request(format!("frame: {e}")))?;
    let (inp, _) = Inputs::read(ctx, req, params)?;
    let cache = inp.cache(&[b"start-state", frame.to_string().as_bytes()]);
    if let Some(v) = cached(&cache) {
        return Ok(Response::json(&v));
    }
    let (source, mapping) = (&inp.opened.0, &inp.opened.1);
    let s = crate::translate::start_at(source, mapping, &inp.rom, &inp.frames()?, frame)?;
    let j = json!({
        "asked": frame,
        "original_frame": s.original_frame,
        "frame": s.frame,
        "state": base64::engine::general_purpose::STANDARD.encode(&s.state),
        "notes": crate::lockstep::notes_json(&s.notes),
    });
    keep(&cache, &j);
    Ok(Response::json(&j))
}

fn lockstep(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let mode = gp_interp::lockstep::Mode::parse(req.param("mode").unwrap_or("resync"))
        .map_err(Fail::bad_request)?;
    let from = req
        .param("from")
        .map(|f| f.parse::<u64>())
        .transpose()
        .map_err(|e| Fail::bad_request(format!("from: {e}")))?;
    let (inp, name) = Inputs::read(ctx, req, params)?;
    let cache = inp.cache(&[
        // The screen measure's version: a cache from before it has none.
        b"screen 1",
        mode.name().as_bytes(),
        from.map(|f| f.to_string()).unwrap_or_default().as_bytes(),
        b"values",
    ]);
    if let Some(v) = cached(&cache) {
        return Ok(Response::json(&v));
    }
    let (source, mapping) = (&inp.opened.0, &inp.opened.1);
    let (ls, mut j, _) = crate::lockstep::run_resumed(
        source,
        &inp.mdir,
        mapping,
        &inp.rom,
        &inp.frames()?,
        mode,
        from,
        true,
        inp.assets.as_deref(),
        None,
        None,
        None,
    )?;
    // Both games' values frame by frame, for the curve chart.
    if let Some(paths) = j["paths"].as_array_mut() {
        for (i, p) in paths.iter_mut().enumerate() {
            let side = |m: bool| -> Vec<Value> {
                ls.values
                    .iter()
                    .map(|(o, md)| {
                        let v = if m { &md[i] } else { &o[i] };
                        gp_interp::scenario::curve_value(v).map_or(Value::Null, Value::from)
                    })
                    .collect()
            };
            p["original"] = Value::from(side(false));
            p["model"] = Value::from(side(true));
        }
    }
    j["recording"] = Value::from(name.as_str());
    keep(&cache, &j);
    Ok(Response::json(&j))
}
