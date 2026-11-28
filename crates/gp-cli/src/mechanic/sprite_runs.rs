//! `GET /api/screen/sprite-runs?id=CLIP`: a corpus clip's sprite runs for
//! the screen page, from the scan `gp picture --sprites` makes
//! ([`crate::picture::run`]): the runs of frames where the original shows a
//! sprite the model doesn't (missing) or the model draws one the original
//! doesn't (extra), each with its frames, the first frame's sprites and the
//! missing sprites' boxes on every frame of it, for the page to box them on
//! the original's side.
//!
//! The scan plays the clip on both games, which takes seconds to minutes,
//! so it runs off the page's thread: the first ask starts it and answers
//! `scanning`, and the page asks again until it answers `done` (or
//! `failed`, with why). Its lines are kept in
//! `$GP_HOME/work/screen-sprites/GAME/MODEL/CLIP.jsonl`, by the clip and the
//! model's version, so opening the list twice scans once, a new server
//! finds them, and a changed model scans again.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use gp_evidence::Paths;
use serde_json::{Value, json};

use super::http::Response;
use super::review::Rebuild;

/// What a scan is asked: the game's package folder, its module, the clip,
/// and the file its lines go to.
pub(crate) struct Asked {
    pub game: PathBuf,
    pub module: Option<PathBuf>,
    pub id: String,
    pub out: PathBuf,
}

/// The scan: `gp picture --sprites` on one clip, writing its lines to
/// `out`. A test gives its own.
pub(crate) type Scan = Arc<dyn Fn(&Paths, &Asked) -> Result<()> + Send + Sync>;

/// The scans running and the ones that failed, by the file they make.
pub(crate) struct SpriteRuns {
    scan: Scan,
    running: Mutex<HashSet<PathBuf>>,
    failed: Mutex<HashMap<PathBuf, String>>,
    /// One scan at a time: they share a run folder and the machine.
    one: Arc<Mutex<()>>,
}

impl Default for SpriteRuns {
    fn default() -> Self {
        Self::with(Arc::new(picture_scan))
    }
}

impl SpriteRuns {
    pub(crate) fn with(scan: Scan) -> Self {
        Self {
            scan,
            running: Mutex::default(),
            failed: Mutex::default(),
            one: Arc::default(),
        }
    }
}

/// The real scan: the clip played through `gp picture --sprites`, the
/// package's scored set, in a run of the server's own.
fn picture_scan(paths: &Paths, a: &Asked) -> Result<()> {
    let name = a
        .game
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("game");
    let args = crate::picture::PictureArgs {
        game: a.game.clone(),
        run: format!("screen-sprites-{name}"),
        module: a.module.clone(),
        model: None,
        assets: None,
        rom: None,
        clip: Some(a.id.clone()),
        top: 0,
        shots: None,
        shots_dir: None,
        sprites: Some(a.out.clone()),
        propose: false,
        line: None,
        apart: false,
    };
    let o = crate::picture::run(&args, paths)?;
    if let Some(e) = o.json["clips"][&a.id]["error"].as_str() {
        bail!("{} can't be played: {e}", a.id);
    }
    Ok(())
}

/// The file a clip's scan on a model version is kept in.
fn kept(paths: &Paths, game: &str, model: &str, id: &str) -> PathBuf {
    let model = if model.is_empty() { "none" } else { model };
    paths
        .home
        .join("work/screen-sprites")
        .join(game)
        .join(model)
        .join(format!("{id}.jsonl"))
}

/// The route. `game` is the server's game, `rb` what it knows of the
/// games checkout.
pub(crate) fn route(
    runs: &Arc<SpriteRuns>,
    paths: &Paths,
    game: &str,
    rb: Option<&Rebuild>,
    id: Option<&str>,
) -> Result<Response> {
    let Some(id) = id.filter(|i| !i.is_empty() && !i.contains(['/', '\\'])) else {
        return Ok(Response::error(400, "id is the corpus clip's"));
    };
    let Some(rb) = rb else {
        return Ok(Response::error(404, "no games checkout: no corpus"));
    };
    if crate::clip_score::corpus_clip(&rb.game, id).is_err() {
        return Ok(Response::error(404, "no such corpus clip"));
    }
    let model = crate::progress_measure::model_key(&rb.game);
    let file = kept(paths, game, &model, id);
    let short = &model[..model.len().min(8)];
    if let Ok(text) = fs::read_to_string(&file) {
        let (frames, list) = runs_of(&text)?;
        return Ok(Response::json(&json!({
            "id": id, "model": short, "state": "done", "frames": frames, "runs": list,
        })));
    }
    // A failure is told once; the next ask scans again.
    if let Some(e) = runs.failed.lock().expect("failed lock").remove(&file) {
        return Ok(Response::json(
            &json!({"id": id, "model": short, "state": "failed", "error": e}),
        ));
    }
    let scanning = json!({"id": id, "model": short, "state": "scanning"});
    if !runs
        .running
        .lock()
        .expect("running lock")
        .insert(file.clone())
    {
        return Ok(Response::json(&scanning));
    }
    let asked = Asked {
        game: rb.game.clone(),
        module: rb.module_now(),
        id: id.to_owned(),
        out: file.with_extension("jsonl.part"),
    };
    let (runs, paths) = (runs.clone(), paths.clone());
    crate::deep_thread::spawn(move || {
        let done = {
            let _one = runs.one.lock().unwrap_or_else(|e| e.into_inner());
            fs::create_dir_all(file.parent().expect("a folder"))
                .map_err(anyhow::Error::from)
                .and_then(|()| (runs.scan)(&paths, &asked))
                .and_then(|()| Ok(fs::rename(&asked.out, &file)?))
        };
        if let Err(e) = done {
            runs.failed
                .lock()
                .expect("failed lock")
                .insert(file.clone(), format!("{e:#}"));
        }
        runs.running.lock().expect("running lock").remove(&file);
    });
    Ok(Response::json(&scanning))
}

/// The codes of a line's missing sprites and the kinds of its extra ones,
/// each sorted once: what makes a run.
fn key_of(l: &Value) -> (Vec<String>, Vec<String>) {
    let mut m: Vec<String> = list(&l["missing"]).iter().map(|x| code(&x[0])).collect();
    m.sort();
    m.dedup();
    let mut e: Vec<String> = list(&l["extra"])
        .iter()
        .map(|x| x[0].as_str().unwrap_or("").to_owned())
        .collect();
    e.sort();
    e.dedup();
    (m, e)
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

fn code(v: &Value) -> String {
    v.as_u64()
        .map_or_else(|| v.to_string(), |c| format!("{c:03x}"))
}

/// A scan's lines (one a compared frame) as its frame count and runs: the
/// frames next to each other with the same missing codes and extra kinds,
/// as `gp picture --sprites` reports them. Each run gives its first and
/// last frame, how many sprites each side shows on its first frame, that
/// frame's missing sprites (code and box) and extra bodies (kind and box),
/// and the missing sprites' boxes on each of its frames.
pub(crate) fn runs_of(text: &str) -> Result<(usize, Vec<Value>)> {
    let lines: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let frame = |l: &Value| l["frame"].as_u64().unwrap_or(0);
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        let k = key_of(l);
        if k.0.is_empty() && k.1.is_empty() {
            i += 1;
            continue;
        }
        let mut j = i;
        while j + 1 < lines.len()
            && frame(&lines[j + 1]) == frame(&lines[j]) + 1
            && key_of(&lines[j + 1]) == k
        {
            j += 1;
        }
        let boxes: Vec<Vec<Value>> = lines[i..=j]
            .iter()
            .map(|x| list(&x["missing"]).iter().map(|m| m[1].clone()).collect())
            .collect();
        out.push(json!({
            "from": frame(l),
            "to": frame(&lines[j]),
            "original": list(&l["original"]).len(),
            "model": list(&l["model"]).len(),
            "missing": list(&l["missing"]).iter().map(|m| json!({"code": code(&m[0]), "box": m[1]})).collect::<Vec<_>>(),
            "extra": list(&l["extra"]).iter().map(|x| json!({"kind": x[0], "box": x[1]})).collect::<Vec<_>>(),
            "boxes": boxes,
        }));
        i = j + 1;
    }
    Ok((lines.len(), out))
}
