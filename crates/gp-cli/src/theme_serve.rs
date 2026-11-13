//! THEMES on the page: the game's theme kit, its themes, the theme
//! commands started as background work, the readability and originality
//! checks with their results, theme clip review and theme refinement.
//!
//! The mechanic page's server ([`crate::mechanic::server`]) hands every
//! path under `/api/themes/` and `/themes/` here. The pane itself is
//! `mechanic/themes/pane.js`.
//!
//! Ported from game-printer's `serve/themes.rs` (the routes `GET
//! /api/themes/{model}`, `POST .../preview` and `POST .../save`, here
//! without `{model}`: the page is one game's) and folded into the one
//! page. What gameprinter's layout changed: the model is the game's in the
//! games checkout (games/GAME), a run is a corpus clip of the game
//! (inputs/NAME.start.presses, then inputs/NAME.presses) where the old
//! route took a baseline or a recording, and everything the tab makes is
//! under `$GP_HOME/work/themes/GAME/`, never the games checkout.
//!
//! | Route | What |
//! |---|---|
//! | `GET /api/themes/all` | The kit, every theme (the game's, the player's, linked folders, made here, refinement rounds) with its last check, the registry's themes, the clips, the style cards, the jobs |
//! | `POST /api/themes/link` | `{dir, art?, sheets?}`: lists a theme folder made elsewhere (the art or sheets it was fitted from, for a refit) |
//! | `POST /api/themes/run` | `{what, theme?, style?, art?, sheets?}`: starts `gp theme kit, check, fit, sheet, generate --dry-run or publish --dry-run` as background work |
//! | `GET /api/themes/jobs/{n}` | A job: its commands, whether it is done, its exit and its output |
//! | `POST /api/themes/preview` | `{layers, drafts, clip, frames}`: the clip drawn with that stack at those frames (PNG, base64), its state digest, and the readability and Style checks (the old route) |
//! | `POST /api/themes/save` | `{theme}` or `{layers}`: a personal theme or the player's stack, under `$GP_HOME/themes/` (the old route) |
//! | `POST /api/themes/review` | `{theme, clip, every}`: theme clip review: the clip drawn with the theme and with the original art, frame for frame |
//! | `GET /api/themes/review/{theme}/{clip}/{side}/{frame}.png` | One drawn frame of a review (`t` themed, `o` the original art) |
//! | `GET`, `POST /api/themes/marks` | A theme's marks on its clip reviews; `{theme, clip, frame, x, y, note, slot?}` adds one, `{theme, delete}` takes one off |
//! | `POST /api/themes/round` | `{theme}`: a refinement round on the marks: a refit when the theme's art is known, then the checks, with what became of each mark |
//!
//! No route here makes a paid call: `generate` always runs with
//! `--dry-run`, and `publish` with `--dry-run`, and the background work
//! runs without RUNWAY_API_KEY.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use gp_evidence::Paths;
use gp_theme::stack::{Places, load_with, personal_stack};
use gp_theme::{Resolved, Scope, Stack, Theme};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mechanic::http::{Request, Response};
use crate::theme::{RunRecorder, ThemeArgs};

/// What a request on one game's page sees.
pub(crate) struct At<'a> {
    pub paths: &'a Paths,
    /// The game's folder name (`donkey-kong`).
    pub game: &'a str,
    /// The game's folder in the games checkout.
    pub dir: &'a Path,
    /// The page's files (mechanic/).
    pub page: &'a Path,
}

/// The most frames a review draws of one clip (each side); a longer clip
/// is drawn every few frames.
const REVIEW_FRAMES: usize = 900;

/// Handles the request when its path is one of THEMES's: `None` otherwise.
pub(crate) fn route(at: &At, req: &Request) -> Option<Result<Response>> {
    let p = req.path.as_str();
    if let Some(name) = p.strip_prefix("/themes/") {
        return Some(Ok(page_file(at, name)));
    }
    let rest = p.strip_prefix("/api/themes/")?;
    let post = req.method == "POST";
    let r = match (post, rest) {
        (false, "all") => all(at).map(|v| Response::json(&v)),
        (true, "link") => link(at, &req.body),
        (true, "run") => start(at, &req.body),
        (false, j) if j.starts_with("jobs/") => job(at, &j["jobs/".len()..]),
        (true, "preview") => preview(at, &req.body),
        (true, "save") => save(at, &req.body),
        (true, "review") => review(at, &req.body).map(|v| Response::json(&v)),
        (false, r) if r.starts_with("review/") => review_png(at, &r["review/".len()..]),
        (false, "marks") => marks_get(at, req.param("theme").unwrap_or("")),
        (true, "marks") => marks_post(at, &req.body),
        (true, "round") => round(at, &req.body),
        _ => Ok(Response::error(404, "no such THEMES route")),
    };
    Some(r.or_else(|e| Ok(Response::error(409, &format!("{e:#}")))))
}

fn page_file(at: &At, name: &str) -> Response {
    let ok = !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
    let kind = if name.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else {
        return Response::error(404, "not here");
    };
    match (ok, fs::read(at.page.join("themes").join(name))) {
        (true, Ok(b)) => Response::bytes(kind, b),
        _ => Response::error(404, "not here"),
    }
}

/// `$GP_HOME/work/themes/GAME/`: everything the tab writes.
fn work(at: &At) -> PathBuf {
    at.paths.home.join("work").join("themes").join(at.game)
}

fn key_of(dir: &Path) -> String {
    let s = dir.to_string_lossy();
    blake3::hash(s.as_bytes()).to_hex()[..12].to_owned()
}

fn safe(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 120
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !s.starts_with('.')
}

/// One theme the tab knows.
struct Entry {
    key: String,
    /// Where it comes from: `game` (games/GAME/themes), `personal`
    /// ($GP_HOME/themes), `linked`, `made` (fitted here) or `round`.
    scope: &'static str,
    dir: PathBuf,
    /// The art or sheets it was fitted from, when known.
    source: Option<(String, PathBuf)>,
    /// For a round: the theme it refines and its number.
    round_of: Option<(String, u32)>,
}

/// The theme folders in a folder.
fn theme_dirs(d: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| gp_theme::same_as::is_theme_dir(p))
        .collect();
    out.sort();
    out
}

/// The tab's own record: linked folders and where made themes came from.
#[derive(Default, serde::Serialize, Deserialize)]
struct Links {
    #[serde(default)]
    link: Vec<Link>,
}

#[derive(Clone, serde::Serialize, Deserialize)]
struct Link {
    dir: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    art: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sheets: Option<PathBuf>,
}

impl Link {
    fn source(&self) -> Option<(String, PathBuf)> {
        self.sheets
            .clone()
            .map(|s| ("sheets".to_owned(), s))
            .or_else(|| self.art.clone().map(|a| ("art".to_owned(), a)))
    }
}

fn links(at: &At) -> Links {
    fs::read_to_string(work(at).join("links.toml"))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_links(at: &At, l: &Links) -> Result<()> {
    write_atomic(&work(at).join("links.toml"), &toml::to_string(l)?)
}

fn entries(at: &At) -> Vec<Entry> {
    let places = Places::for_model(at.dir, Some(&at.paths.home));
    let mut out = Vec::new();
    let mut push = |scope, dir: PathBuf, source, round_of| {
        if !out.iter().any(|e: &Entry| e.dir == dir) {
            out.push(Entry {
                key: key_of(&dir),
                scope,
                dir,
                source,
                round_of,
            });
        }
    };
    for d in places.baked.iter().flat_map(|b| theme_dirs(b)) {
        push("game", d, None, None);
    }
    for d in places.personal.iter().flat_map(|b| theme_dirs(b)) {
        push("personal", d, None, None);
    }
    let l = links(at);
    for k in &l.link {
        if gp_theme::same_as::is_theme_dir(&k.dir) {
            let scope = if k.dir.starts_with(work(at).join("made")) {
                "made"
            } else {
                "linked"
            };
            push(scope, k.dir.clone(), k.source(), None);
        }
    }
    // Rounds: rounds/KEY/N/theme.
    for r in fs::read_dir(work(at).join("rounds"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let of = r.file_name().to_string_lossy().into_owned();
        let mut ns: Vec<u32> = fs::read_dir(r.path())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .collect();
        ns.sort_unstable();
        for n in ns {
            let d = r.path().join(n.to_string()).join("theme");
            if gp_theme::same_as::is_theme_dir(&d) {
                push("round", d, None, Some((of.clone(), n)));
            }
        }
    }
    out
}

fn entry(at: &At, key: &str) -> Result<Entry> {
    entries(at)
        .into_iter()
        .find(|e| e.key == key)
        .ok_or_else(|| anyhow!("no theme `{key}` on this page"))
}

/// The theme's id and title from its theme.toml, and its problems.
fn theme_info(e: &Entry) -> Value {
    let loaded = gp_theme::same_as::target(&e.dir).and_then(|d| Ok((Theme::load(&d)?, d)));
    match loaded {
        Ok((t, d)) => {
            let problems = t.check(&d);
            json!({
                "id": t.id, "title": t.title, "license": t.license,
                "for": t.for_models,
                "restyles": t.restyle.len(), "sounds": t.sound.len(),
                "palettes": t.palette.len(),
                "problem": (!problems.is_empty()).then(|| format!("{problems:?}")),
            })
        }
        Err(e) => json!({"problem": e}),
    }
}

/// The game's corpus clips: inputs/NAME.presses with its start.
fn clips(at: &At) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = fs::read_dir(at.dir.join("inputs"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_str()?.to_owned();
            let stem = n.strip_suffix(".presses")?;
            if stem.ends_with(".start") {
                return None;
            }
            let frames = fs::read_to_string(e.path())
                .ok()?
                .lines()
                .find_map(|l| l.strip_prefix("frames ")?.trim().parse().ok())
                .unwrap_or(0);
            Some((stem.to_owned(), frames))
        })
        .collect();
    out.sort();
    out
}

fn kit(at: &At) -> Value {
    let read = |f: &Path| -> Option<crate::theme_kit::Kit> {
        toml::from_str(&fs::read_to_string(f).ok()?).ok()
    };
    let game_file = at.dir.join("themes/kit").join(crate::theme_kit::KIT_FILE);
    let made_file = work(at).join("kit").join(crate::theme_kit::KIT_FILE);
    let (file, k) = match (read(&made_file), read(&game_file)) {
        (Some(k), _) => (made_file, Some(k)),
        (None, k) => (game_file, k),
    };
    let Some(k) = k else {
        return json!({"file": null});
    };
    let local = crate::theme_kit::local_dir(at.paths, &k.game);
    let model_slots = crate::theme::read_model(at.dir)
        .ok()
        .map(|(m, _)| crate::theme_kit::kit_of(&m, &k.game).slots);
    json!({
        "file": file,
        "game": k.game, "model": k.model, "screen": k.screen, "slots": k.slots,
        "current": model_slots.as_deref().map(|s| s == k.slots),
        "local": local.is_dir().then(|| local.clone()),
        "reference": local.join("reference.png").is_file(),
        "art": k.art.iter().map(|a| json!({
            "id": a.id, "kind": a.kind, "role": a.role, "size": a.size, "frames": a.frames,
            "label": a.label, "description": a.description, "colors": a.colors,
        })).collect::<Vec<_>>(),
        "sound": k.sound.iter().map(|s| json!({
            "role": s.role, "kind": s.kind, "cue": s.cue, "frames": s.frames,
            "loops": s.loops, "description": s.description,
        })).collect::<Vec<_>>(),
    })
}

/// The content packages the person's registry offers for this game.
fn registry(at: &At) -> Value {
    let catalog = at.paths.home.join("registry").join("catalog.toml");
    let suffix = format!("/{}", at.game);
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
    json!({"catalog": catalog.is_file().then_some(catalog), "themes": list})
}

fn styles(at: &At) -> Vec<Value> {
    let mut out = Vec::new();
    for (scope, d) in [
        ("personal", crate::theme_style::personal_dir(at.paths)),
        ("shared", crate::theme_style::shared_dir()),
    ] {
        let mut names: Vec<String> = fs::read_dir(&d)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_str()?.to_owned();
                n.strip_suffix(".toml").map(str::to_owned)
            })
            .collect();
        names.sort();
        for n in names {
            if !out.iter().any(|v: &Value| v["name"] == json!(n)) {
                out.push(json!({"name": n, "scope": scope}));
            }
        }
    }
    out
}

/// A check's result as the tab keeps it: the JSON `gp theme check
/// --original --json` printed, with when.
fn result_of(at: &At, key: &str) -> Value {
    let d = work(at).join("results").join(key);
    let read = |n: &str| {
        fs::read_to_string(d.join(n))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    };
    let publish = fs::read_to_string(d.join("publish.txt")).ok();
    json!({"check": read("check.json"), "publish": publish})
}

fn all(at: &At) -> Result<Value> {
    let (m, _) = crate::theme::read_model(at.dir)?;
    let themes: Vec<Value> = entries(at)
        .iter()
        .map(|e| {
            let mut v = theme_info(e);
            v["key"] = json!(e.key);
            v["scope"] = json!(e.scope);
            v["dir"] = json!(e.dir);
            v["source"] = json!(e.source.as_ref().map(|(k, p)| json!({"kind": k, "dir": p})));
            v["round_of"] = json!(
                e.round_of
                    .as_ref()
                    .map(|(k, n)| json!({"theme": k, "n": n}))
            );
            v["result"] = result_of(at, &e.key);
            v["marks"] = json!(read_marks(at, &e.key).len());
            v["rounds"] = json!(rounds_of(at, &e.key));
            v
        })
        .collect();
    let games = at.dir.parent().unwrap_or(at.dir);
    let index = games
        .join(gp_package::index::REPO_DIR)
        .join(format!("{}.toml", at.game));
    Ok(json!({
        "game": at.game,
        "dir": at.dir,
        "model": m.model.name,
        "title": m.model.title,
        "work": work(at),
        "kit": kit(at),
        "themes": themes,
        "registry": registry(at),
        "index": index.is_file().then_some(index),
        "clips": clips(at).into_iter().map(|(n, f)| json!({"name": n, "frames": f})).collect::<Vec<_>>(),
        "styles": styles(at),
        "jobs": jobs(at),
    }))
}

#[derive(Deserialize)]
struct LinkBody {
    dir: PathBuf,
    #[serde(default)]
    art: Option<PathBuf>,
    #[serde(default)]
    sheets: Option<PathBuf>,
}

fn link(at: &At, body: &[u8]) -> Result<Response> {
    let b: LinkBody =
        serde_json::from_slice(body).context("the body isn't {dir, art?, sheets?}")?;
    let dir = fs::canonicalize(&b.dir).with_context(|| format!("{}", b.dir.display()))?;
    if !gp_theme::same_as::is_theme_dir(&dir) {
        bail!("{} holds no theme.toml", dir.display());
    }
    for s in [&b.art, &b.sheets].into_iter().flatten() {
        if !s.is_dir() {
            bail!("{} isn't a folder", s.display());
        }
    }
    let mut l = links(at);
    l.link.retain(|k| k.dir != dir);
    l.link.push(Link {
        dir: dir.clone(),
        art: b.art,
        sheets: b.sheets,
    });
    save_links(at, &l)?;
    Ok(Response::json(&json!({"key": key_of(&dir)})))
}

// ---- Background work ----

/// One command of a job: its arguments after `gp`, and the file its
/// standard output goes to (the job's log when none).
struct Step {
    args: Vec<String>,
    out: Option<PathBuf>,
}

/// What a job does after its commands, in its thread, with their exits.
type Then = Box<dyn FnOnce(&[i32]) + Send>;

fn jobs_dir(at: &At) -> PathBuf {
    work(at).join("jobs")
}

fn jobs(at: &At) -> Vec<Value> {
    let mut v: Vec<(u32, Value)> = fs::read_dir(jobs_dir(at))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n: u32 = e
                .file_name()
                .to_str()?
                .strip_suffix(".json")?
                .parse()
                .ok()?;
            let j: Value = serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok()?;
            Some((n, j))
        })
        .collect();
    v.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    v.into_iter().take(30).map(|(_, j)| j).collect()
}

fn job(at: &At, n: &str) -> Result<Response> {
    let n: u32 = n.parse().context("a job is a number")?;
    let d = jobs_dir(at);
    let state: Value = serde_json::from_str(
        &fs::read_to_string(d.join(format!("{n}.json"))).context("no such job")?,
    )?;
    let log = fs::read_to_string(d.join(format!("{n}.log"))).unwrap_or_default();
    // The end of a long log.
    let log = if log.len() > 60_000 {
        let mut s = log.len() - 60_000;
        while !log.is_char_boundary(s) {
            s += 1;
        }
        format!("...\n{}", &log[s..])
    } else {
        log
    };
    Ok(Response::json(&json!({"job": state, "log": log})))
}

/// Starts the steps in the background, one after another, each `gp` with
/// the games checkout given and no Runway key. Stops at the first that
/// fails unless `go_on`. `then` runs after, in the same thread.
fn spawn_job(
    at: &At,
    what: &str,
    theme: Option<&str>,
    steps: Vec<Step>,
    go_on: bool,
    then: Option<Then>,
) -> Result<u32> {
    let d = jobs_dir(at);
    fs::create_dir_all(&d)?;
    let n = fs::read_dir(&d)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".json")?
                .parse::<u32>()
                .ok()
        })
        .max()
        .map_or(1, |m| m + 1);
    let exe = std::env::current_exe()?;
    let games = at.dir.parent().unwrap_or(at.dir).to_owned();
    let state_file = d.join(format!("{n}.json"));
    let log_file = d.join(format!("{n}.log"));
    let commands: Vec<String> = steps
        .iter()
        .map(|s| format!("gp {}", s.args.join(" ")))
        .collect();
    let mut state = json!({
        "n": n, "what": what, "theme": theme, "commands": commands,
        "done": false, "exits": [], "started": now(),
    });
    write_atomic(&state_file, &serde_json::to_string_pretty(&state)?)?;
    let home = at.paths.home.clone();
    std::thread::spawn(move || {
        let mut exits = Vec::new();
        for s in &steps {
            let log = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file);
            let Ok(mut log) = log else { break };
            use std::io::Write as _;
            let _ = writeln!(log, "$ gp {}", s.args.join(" "));
            let stdout = match &s.out {
                Some(f) => fs::File::create(f).map(Stdio::from),
                None => log.try_clone().map(Stdio::from),
            };
            let code = match (stdout, log.try_clone()) {
                (Ok(out), Ok(err)) => Command::new(&exe)
                    .args(&s.args)
                    .current_dir(crate::serve::repo_root())
                    .env("GP_HOME", &home)
                    .env("GP_GAMES_DIR", &games)
                    .env_remove("RUNWAY_API_KEY")
                    .stdin(Stdio::null())
                    .stdout(out)
                    .stderr(err)
                    .status()
                    .ok()
                    .and_then(|s| s.code())
                    .unwrap_or(-1),
                _ => -1,
            };
            let _ = writeln!(log, "(exit {code})");
            exits.push(code);
            state["exits"] = json!(exits);
            let _ = write_atomic(
                &state_file,
                &serde_json::to_string_pretty(&state).unwrap_or_default(),
            );
            if code != 0 && !go_on {
                break;
            }
        }
        if let Some(f) = then {
            f(&exits);
        }
        state["done"] = json!(true);
        state["ended"] = json!(now());
        let _ = write_atomic(
            &state_file,
            &serde_json::to_string_pretty(&state).unwrap_or_default(),
        );
    });
    Ok(n)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Deserialize)]
struct RunBody {
    what: String,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    style: Option<String>,
    #[serde(default)]
    art: Option<PathBuf>,
    #[serde(default)]
    sheets: Option<PathBuf>,
    /// A fitted theme's folder name under made/.
    #[serde(default)]
    name: Option<String>,
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The steps of `gp theme check --original` on a theme, its JSON kept.
fn check_steps(at: &At, theme_dir: &Path, key: &str) -> Result<Vec<Step>> {
    let rd = work(at).join("results").join(key);
    fs::create_dir_all(&rd)?;
    Ok(vec![Step {
        args: vec![
            "--json".into(),
            "theme".into(),
            "check".into(),
            s(at.dir),
            "--theme".into(),
            s(theme_dir),
            "--original".into(),
        ],
        out: Some(rd.join("check.json")),
    }])
}

fn start(at: &At, body: &[u8]) -> Result<Response> {
    let b: RunBody = serde_json::from_slice(body).context("the body isn't {what, ...}")?;
    let w = work(at);
    let theme = b.theme.as_deref().map(|k| entry(at, k)).transpose()?;
    let need_theme = || theme.as_ref().context("name a theme");
    let style = || -> Result<String> {
        let st = b.style.clone().context("name a style card")?;
        if !safe(&st) {
            bail!("`{st}` isn't a style card's name");
        }
        Ok(st)
    };
    let (steps, go_on): (Vec<Step>, bool) = match b.what.as_str() {
        // The kit's shareable part goes under the tab's folder, never into
        // the games checkout; the local part to $GP_HOME/work/theme-kits.
        "kit" => {
            let mut args = vec![
                "theme".into(),
                "kit".into(),
                s(at.dir),
                "-o".into(),
                s(&w.join("kit")),
            ];
            let t = crate::theme_template::load(at.dir)?;
            if let Some(r) = crate::theme_template::find_rom(&t, None, at.paths)? {
                args.extend(["--rom".into(), s(&r.path)]);
            }
            (vec![Step { args, out: None }], false)
        }
        "check" => {
            let e = need_theme()?;
            (check_steps(at, &e.dir, &e.key)?, false)
        }
        "sheet" => {
            let st = style()?;
            let out = w.join("sheets").join(&st);
            (
                vec![Step {
                    args: vec![
                        "theme".into(),
                        "sheet".into(),
                        s(at.dir),
                        "--style".into(),
                        st,
                        "-o".into(),
                        s(&out),
                        "--force".into(),
                    ],
                    out: None,
                }],
                false,
            )
        }
        // A dry run only: the calls and their cost, none made.
        "generate" => {
            let st = style()?;
            let mut args = vec![
                "theme".into(),
                "generate".into(),
                s(at.dir),
                "--style".into(),
                st.clone(),
                "--dry-run".into(),
                "--run".into(),
                s(&w.join("runs").join(&st)),
            ];
            if let Some(sh) = &b.sheets {
                args.extend(["--sheets".into(), s(sh)]);
            }
            (vec![Step { args, out: None }], false)
        }
        "fit" => {
            let name = b.name.clone().unwrap_or_else(|| "fit".into());
            if !safe(&name) {
                bail!("`{name}` isn't a folder name");
            }
            let out = w.join("made").join(&name);
            let mut args = vec!["theme".into(), "fit".into(), s(at.dir)];
            match (&b.sheets, &b.art) {
                (Some(sh), _) => args.extend(["--sheets".into(), s(sh)]),
                (None, Some(a)) => args.extend(["--art".into(), s(a)]),
                (None, None) => bail!("give the art folder or the sheets folder to fit from"),
            }
            args.extend([
                "-o".into(),
                s(&out),
                "--contact".into(),
                s(&w.join("made").join(format!("{name}.contact.png"))),
            ]);
            // Listed now, so its checks and review are one click away once
            // it is written.
            let mut l = links(at);
            l.link.retain(|k| k.dir != out);
            l.link.push(Link {
                dir: out.clone(),
                art: b.art.clone(),
                sheets: b.sheets.clone(),
            });
            save_links(at, &l)?;
            let mut steps = vec![Step { args, out: None }];
            steps.extend(check_steps(at, &out, &key_of(&out))?);
            (steps, false)
        }
        // A dry run only: every check and the package scan, then what would
        // be published. Nothing goes to a store, a mirror or an index.
        "publish" => {
            let e = need_theme()?;
            let rd = w.join("results").join(&e.key);
            fs::create_dir_all(&rd)?;
            (
                vec![Step {
                    args: vec![
                        "theme".into(),
                        "publish".into(),
                        s(&e.dir),
                        "--game".into(),
                        at.game.to_owned(),
                        "--dry-run".into(),
                    ],
                    out: Some(rd.join("publish.txt")),
                }],
                false,
            )
        }
        other => bail!(
            "`{other}` isn't one of kit, check, sheet, generate (a dry run), fit, publish (a dry run)"
        ),
    };
    let n = spawn_job(
        at,
        &b.what,
        theme.as_ref().map(|e| e.key.as_str()),
        steps,
        go_on,
        None,
    )?;
    Ok(Response::json(&json!({"job": n})))
}

// ---- The old preview and save routes ----

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewBody {
    layers: Vec<gp_theme::format::Layer>,
    /// Edited themes, by id, drawn in place of their saved files.
    #[serde(default)]
    drafts: BTreeMap<String, Theme>,
    /// A corpus clip of the game (gameprinter; the old route took a
    /// baseline or a recording).
    clip: String,
    /// Frames of the clip to draw, at most 16.
    frames: Vec<u64>,
}

/// The stack a request names, with its drafts in place.
fn stack_of(
    p: &Places,
    model: &str,
    layers: &[gp_theme::format::Layer],
    drafts: &BTreeMap<String, Theme>,
) -> Result<Resolved> {
    let stack = Stack {
        format: gp_theme::FORMAT_VERSION.into(),
        model: Some(model.into()),
        layer: layers.to_vec(),
    };
    let mut r = gp_theme::stack::resolve(&stack, p, model).map_err(anyhow::Error::msg)?;
    for l in &mut r.layers {
        if let Some(d) = drafts.get(&l.theme.id) {
            if d.id != l.theme.id {
                bail!("a draft keeps its theme's id");
            }
            *l = load_with(&l.dir, l.scope, d.clone()).map_err(anyhow::Error::msg)?;
        }
    }
    Ok(r)
}

/// A corpus clip's inputs: the start (from power-on to the clip) and the
/// clip.
fn clip_inputs(
    at: &At,
    clip: &str,
) -> Result<(Vec<gp_runtime::InputFrame>, Vec<gp_runtime::InputFrame>)> {
    if !safe(clip) {
        bail!("`{clip}` isn't a clip's name");
    }
    let dir = at.dir.join("inputs");
    let body = fs::read_to_string(dir.join(format!("{clip}.presses")))
        .with_context(|| format!("no clip {clip} in {}", dir.display()))?;
    let start = fs::read_to_string(dir.join(format!("{clip}.start.presses")))
        .map(|t| crate::theme::presses_frames(&t))
        .unwrap_or_default();
    Ok((start, crate::theme::presses_frames(&body)))
}

fn preview(at: &At, body: &[u8]) -> Result<Response> {
    let b: PreviewBody =
        serde_json::from_slice(body).context("the body isn't a preview request")?;
    if b.frames.len() > 16 {
        bail!("at most 16 frames a preview");
    }
    let (m, dir) = crate::theme::read_model(at.dir)?;
    let places = Places::for_model(&dir, Some(&at.paths.home));
    let theme = stack_of(&places, &m.model.name, &b.layers, &b.drafts)?;
    let assets = at
        .paths
        .home
        .join("work")
        .join("assets")
        .join(m.model.source.clone().unwrap_or_default());
    let bundle = crate::theme_run::load_bundle(&dir, &assets)?;
    let spec = gp_interp::compile::compile(&m, &bundle.assets).map_err(anyhow::Error::msg)?;
    let art =
        gp_look::present::art_themed(&m, &spec, &assets, &theme).map_err(anyhow::Error::msg)?;
    let readability = gp_theme::readability::check(&art.theme.samples, &theme);
    let style = gp_theme::style::Reference::of(&theme).map(|r| {
        use gp_theme::style::StyleMeasure as _;
        gp_theme::style::Deterministic.measure(&art.theme.subjects, &r)
    });
    let mut rt = crate::theme_run::Run::open(&bundle, &dir, at.paths)?;
    let renderer = crate::theme::Renderer::new(&m, &art, rt.spec())?;
    let want: std::collections::BTreeSet<u64> = b.frames.iter().copied().collect();
    let (start, clip) = clip_inputs(at, &b.clip)?;
    for f in &start {
        rt.advance(f).map_err(anyhow::Error::msg)?;
    }
    let mut hashes = vec![rt.frame_hash()];
    let mut frames = Vec::new();
    for (i, f) in clip.iter().enumerate() {
        rt.advance(f).map_err(anyhow::Error::msg)?;
        hashes.push(rt.frame_hash());
        if want.contains(&(i as u64 + 1)) {
            let png = renderer.frame(rt.spec(), rt.world()).png();
            frames.push(json!({
                "frame": i + 1,
                "png": base64::engine::general_purpose::STANDARD.encode(png),
            }));
        }
    }
    let digest = gp_evidence::Hash::of(&gp_evidence::frame_hashes::encode(&hashes));
    Ok(Response::json(&json!({
        "model": m.model.name,
        "layers": theme.describe(),
        "frames": frames,
        "length": hashes.len() - 1,
        "digest": digest,
        "readability": readability,
        "style": style,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveBody {
    #[serde(default)]
    theme: Option<Theme>,
    /// The player's layers for the model; baked layers are the game's and
    /// aren't saved here.
    #[serde(default)]
    layers: Option<Vec<gp_theme::format::Layer>>,
}

fn save(at: &At, body: &[u8]) -> Result<Response> {
    let b: SaveBody = serde_json::from_slice(body).context("the body isn't a theme or a stack")?;
    let (m, dir) = crate::theme::read_model(at.dir)?;
    let personal = at.paths.home.join("themes");
    let mut wrote = Vec::new();
    if let Some(t) = b.theme {
        if !gp_theme::format::is_name(&t.id) {
            bail!("`{}` isn't a theme id", t.id);
        }
        let tdir = personal.join(&t.id);
        let problems = t.check(&tdir);
        if !problems.is_empty() {
            bail!("the theme doesn't check: {problems:?}");
        }
        write_atomic(&tdir.join(gp_theme::format::THEME_FILE), &t.to_text())?;
        wrote.push(tdir.join(gp_theme::format::THEME_FILE));
    }
    if let Some(layers) = b.layers {
        if layers.iter().any(|l| l.scope != Scope::Personal) {
            bail!("only personal layers are saved; baked ones are the game's");
        }
        let st = Stack {
            format: gp_theme::FORMAT_VERSION.into(),
            model: Some(m.model.name.clone()),
            layer: layers,
        };
        // It must resolve before it's saved.
        gp_theme::stack::resolve(
            &st,
            &Places::for_model(&dir, Some(&at.paths.home)),
            &m.model.name,
        )
        .map_err(anyhow::Error::msg)?;
        let f = personal_stack(&personal, &m.model.name);
        write_atomic(&f, &st.to_text())?;
        wrote.push(f);
    }
    Ok(Response::json(&json!({ "wrote": wrote })))
}

/// Writes by a temporary file and a rename, so a reader never sees half.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().context("a file in a folder")?;
    fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
    let tmp = dir.join(format!(
        ".{}.{}.partial",
        std::process::id(),
        path.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ));
    fs::write(&tmp, text)
        .and_then(|()| fs::rename(&tmp, path))
        .with_context(|| path.display().to_string())
}

// ---- Theme clip review ----

#[derive(Deserialize)]
struct ReviewBody {
    theme: String,
    clip: String,
    #[serde(default)]
    every: Option<usize>,
}

fn review_dir(at: &At, key: &str, clip: &str) -> PathBuf {
    work(at).join("review").join(key).join(clip)
}

/// The newest change to a theme's files, so a review drawn before it is
/// drawn again.
fn theme_stamp(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .max()
        .unwrap_or(0)
}

/// Theme clip review: the clip played on the game's model and drawn twice
/// from the same run, once with the theme and once with the original art,
/// so frame N of one side is frame N of the other. The frames go to
/// `$GP_HOME/work/themes/GAME/review/THEME/CLIP/{t,o}/N.png`; drawn once
/// per theme change.
fn review(at: &At, body: &[u8]) -> Result<Value> {
    let b: ReviewBody = serde_json::from_slice(body).context("the body isn't {theme, clip}")?;
    let e = entry(at, &b.theme)?;
    let (start, clip) = clip_inputs(at, &b.clip)?;
    let every = b
        .every
        .unwrap_or(1)
        .max(clip.len().div_ceil(REVIEW_FRAMES))
        .max(1);
    let out = review_dir(at, &e.key, &b.clip);
    let stamp = theme_stamp(&e.dir);
    let done_file = out.join("review.json");
    if let Some(v) = fs::read_to_string(&done_file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        && v["stamp"] == json!(stamp)
        && v["every"] == json!(every)
    {
        return Ok(v);
    }
    let mut rec = RunRecorder;
    let targs = ThemeArgs {
        theme_dirs: vec![e.dir.clone()],
        allow_unreadable: true,
        ..ThemeArgs::default()
    };
    let (m, theme, art, bundle) = crate::theme::themed(at.dir, &targs, None, at.paths, &mut rec)?;
    let (_, _, oart, _) =
        crate::theme::themed(at.dir, &ThemeArgs::default(), None, at.paths, &mut rec)?;
    let readability = gp_theme::readability::check(&art.theme.samples, &theme);
    let mut rt = crate::theme_run::Run::open(&bundle, at.dir, at.paths)?;
    let (rt_t, rt_o) = (
        crate::theme::Renderer::new(&m, &art, rt.spec())?,
        crate::theme::Renderer::new(&m, &oart, rt.spec())?,
    );
    let (mut sh_t, mut sh_o) = (
        crate::theme::Shown::new(&art)?,
        crate::theme::Shown::new(&oart)?,
    );
    for side in ["t", "o"] {
        fs::create_dir_all(out.join(side))?;
    }
    sh_t.step(&rt, &[]);
    sh_o.step(&rt, &[]);
    for f in &start {
        let (ev, _) = rt.advance(f).map_err(anyhow::Error::msg)?;
        sh_t.step(&rt, &ev);
        sh_o.step(&rt, &ev);
    }
    let mut hashes = vec![rt.frame_hash()];
    let mut frames = Vec::new();
    for (i, f) in clip.iter().enumerate() {
        let (ev, _) = rt.advance(f).map_err(anyhow::Error::msg)?;
        hashes.push(rt.frame_hash());
        sh_t.step(&rt, &ev);
        sh_o.step(&rt, &ev);
        let n = i + 1;
        if n % every == 0 || n == 1 || n == clip.len() {
            fs::write(
                out.join("t").join(format!("{n}.png")),
                sh_t.frame(&rt_t, &rt).png(),
            )?;
            fs::write(
                out.join("o").join(format!("{n}.png")),
                sh_o.frame(&rt_o, &rt).png(),
            )?;
            frames.push(n);
        }
    }
    let digest = gp_evidence::Hash::of(&gp_evidence::frame_hashes::encode(&hashes));
    let v = json!({
        "theme": e.key,
        "clip": b.clip,
        "every": every,
        "stamp": stamp,
        "frames": frames,
        "length": clip.len(),
        "start": start.len(),
        "digest": digest,
        "layers": theme.describe(),
        "readability": readability,
        "not_run": rt.not_run,
        "base": format!("api/themes/review/{}/{}", e.key, b.clip),
    });
    write_atomic(&done_file, &serde_json::to_string(&v)?)?;
    Ok(v)
}

fn review_png(at: &At, rest: &str) -> Result<Response> {
    // KEY/CLIP/SIDE/N.png
    let parts: Vec<&str> = rest.split('/').collect();
    let [key, clip, side, file] = parts[..] else {
        return Ok(Response::error(404, "not a review frame"));
    };
    if !safe(key) || !safe(clip) || !matches!(side, "t" | "o") || !safe(file) {
        return Ok(Response::error(404, "not a review frame"));
    }
    match fs::read(review_dir(at, key, clip).join(side).join(file)) {
        Ok(b) => Ok(Response::bytes("image/png", b)),
        Err(_) => Ok(Response::error(404, "that frame isn't drawn")),
    }
}

// ---- Marks and refinement rounds ----

fn marks_file(at: &At, key: &str) -> PathBuf {
    work(at).join("marks").join(format!("{key}.json"))
}

fn read_marks(at: &At, key: &str) -> Vec<Value> {
    fs::read_to_string(marks_file(at, key))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn marks_get(at: &At, key: &str) -> Result<Response> {
    if !safe(key) {
        bail!("name a theme");
    }
    Ok(Response::json(&json!({"marks": read_marks(at, key)})))
}

#[derive(Deserialize)]
struct MarkBody {
    theme: String,
    #[serde(default)]
    delete: Option<u64>,
    #[serde(default)]
    clip: Option<String>,
    #[serde(default)]
    frame: Option<usize>,
    #[serde(default)]
    x: Option<i64>,
    #[serde(default)]
    y: Option<i64>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    slot: Option<String>,
}

/// The kit's slot of the entity drawn at (x, y) on a clip's frame, by its
/// box: the slot the mark is most likely about.
fn slot_at(at: &At, clip: &str, frame: usize, x: i64, y: i64) -> Result<Option<String>> {
    let (m, dir) = crate::theme::read_model(at.dir)?;
    let assets = at
        .paths
        .home
        .join("work")
        .join("assets")
        .join(m.model.source.clone().unwrap_or_default());
    let bundle = crate::theme_run::load_bundle(&dir, &assets)?;
    let mut rt = crate::theme_run::Run::open(&bundle, &dir, at.paths)?;
    let (start, body) = clip_inputs(at, clip)?;
    for f in start.iter().chain(body.iter().take(frame)) {
        rt.advance(f).map_err(anyhow::Error::msg)?;
    }
    let names = crate::theme_kit::names_of(&m);
    let (spec, w) = (rt.spec(), rt.world());
    // The body whose box is nearest the point, within 16 px; the smaller
    // box when two hold it.
    let mut best: Option<((i64, i64), String)> = None;
    for b in w.bodies.iter().filter(|b| b.active()) {
        let Some(k) = spec.kinds.get(b.kind) else {
            continue;
        };
        let (bx, by) = (b.x.floor(), b.y.floor());
        let (bw, bh) = (k.size[0].floor().max(1), k.size[1].floor().max(1));
        let dx = (bx - x).max(x - (bx + bw)).max(0);
        let dy = (by - y).max(y - (by + bh)).max(0);
        let d = dx.max(dy);
        if d > 16 {
            continue;
        }
        let Some(e) = m.entities.get(&k.name) else {
            continue;
        };
        let id = crate::theme_kit::entity_id(&k.name, &e.core.role, &names);
        let rank = (d, bw * bh);
        if best.as_ref().is_none_or(|(r, _)| rank < *r) {
            best = Some((rank, id));
        }
    }
    Ok(best.map(|(_, id)| id))
}

fn marks_post(at: &At, body: &[u8]) -> Result<Response> {
    let b: MarkBody = serde_json::from_slice(body).context("the body isn't a mark")?;
    let e = entry(at, &b.theme)?;
    let mut marks = read_marks(at, &e.key);
    if let Some(id) = b.delete {
        marks.retain(|m| m["id"] != json!(id));
    } else {
        let clip = b.clip.context("a mark is on a clip")?;
        let frame = b.frame.context("a mark is on a frame")?;
        let (x, y) = (b.x.unwrap_or(-1), b.y.unwrap_or(-1));
        let slot = match b.slot.filter(|s| !s.is_empty()) {
            Some(s) => Some(s),
            None if x >= 0 && y >= 0 => slot_at(at, &clip, frame, x, y).unwrap_or(None),
            None => None,
        };
        let id = marks
            .iter()
            .filter_map(|m| m["id"].as_u64())
            .max()
            .map_or(1, |m| m + 1);
        marks.push(json!({
            "id": id, "clip": clip, "frame": frame, "x": x, "y": y,
            "note": b.note.unwrap_or_default(), "slot": slot, "at": now(),
        }));
    }
    write_atomic(
        &marks_file(at, &e.key),
        &serde_json::to_string_pretty(&marks)?,
    )?;
    Ok(Response::json(&json!({"marks": marks})))
}

fn rounds_dir(at: &At, key: &str) -> PathBuf {
    work(at).join("rounds").join(key)
}

/// The rounds kept for a theme, newest last: `rounds/KEY/N/round.json`.
fn rounds_of(at: &At, key: &str) -> Vec<Value> {
    let mut v: Vec<(u32, Value)> = fs::read_dir(rounds_dir(at, key))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n: u32 = e.file_name().to_str()?.parse().ok()?;
            let r = fs::read_to_string(e.path().join("round.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_else(|| json!({"n": n, "running": true}));
            Some((n, r))
        })
        .collect();
    v.sort_by_key(|(n, _)| *n);
    v.into_iter().map(|(_, r)| r).collect()
}

#[derive(Deserialize)]
struct RoundBody {
    theme: String,
}

/// The readability check's group a kit role is read in.
fn group_of(role: &str) -> &'static str {
    match role {
        "player" => "player",
        "enemy" => "enemies",
        "hazard" => "hazards",
        "hud" | "menu" => "text",
        _ => "",
    }
}

/// What became of each mark after a round's checks: the mark's slot's
/// readability group, originality score and geometry flags.
fn judge_marks(marks: &[Value], check: &Value, roles: &BTreeMap<String, String>) -> Vec<Value> {
    let scores = check["originality"]["scores"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let flags = check["geometry"]["flags"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    marks
        .iter()
        .map(|mk| {
            let slot = mk["slot"].as_str().unwrap_or("");
            let score = scores.iter().find(|s| s["what"].as_str() == Some(slot));
            let refused_flags: Vec<&Value> = flags
                .iter()
                .filter(|f| f["slot"].as_str() == Some(slot) && f["verdict"] == json!("refuse"))
                .collect();
            let warn = flags
                .iter()
                .filter(|f| f["slot"].as_str() == Some(slot))
                .count();
            let group = roles.get(slot).map_or("", |r| group_of(r));
            let unreadable = !group.is_empty()
                && check["readability"]["groups"].as_array().is_some_and(|g| {
                    g.iter()
                        .any(|x| x["group"] == json!(group) && x["verdict"] == json!("refuse"))
                });
            let state = if slot.is_empty() {
                "no slot named: see the checks"
            } else if unreadable {
                "still hard to read: the readability check refuses its group"
            } else if score.is_some_and(|s| s["refused"] == json!(true)) {
                "still too close to the original"
            } else if !refused_flags.is_empty() {
                "still refused by the geometry check"
            } else {
                "passes its checks"
            };
            json!({
                "mark": mk["id"], "slot": slot, "note": mk["note"], "state": state,
                "originality": score.map(|s| s["value"].clone()),
                "geometry_flags": warn,
            })
        })
        .collect()
}

/// A refinement round on a theme's marks, as background work: the theme
/// fitted again into `rounds/KEY/N/theme` from the art or sheets it came
/// from (when the tab knows them; else the round checks the theme as it
/// is), retry prompts for the marked cells when its sheets are the tab's
/// own, then `gp theme check --original` on the result. The round's
/// record says what became of each mark.
fn round(at: &At, body: &[u8]) -> Result<Response> {
    let b: RoundBody = serde_json::from_slice(body).context("the body isn't {theme}")?;
    let e = entry(at, &b.theme)?;
    // A round of a round refines the theme it came from.
    let (base_key, base) = match &e.round_of {
        Some((k, _)) => (k.clone(), entry(at, k)?),
        None => (e.key.clone(), e),
    };
    let marks = read_marks(at, &b.theme);
    let rd = rounds_dir(at, &base_key);
    let n = fs::read_dir(&rd)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|x| x.file_name().to_str()?.parse::<u32>().ok())
        .max()
        .map_or(1, |m| m + 1);
    let dir = rd.join(n.to_string());
    fs::create_dir_all(&dir)?;
    let mut steps = Vec::new();
    let mut did = Vec::new();
    let theme_dir = match &base.source {
        Some((kind, src)) => {
            let out = dir.join("theme");
            steps.push(Step {
                args: vec![
                    "theme".into(),
                    "fit".into(),
                    s(at.dir),
                    format!("--{kind}"),
                    s(src),
                    "-o".into(),
                    s(&out),
                    "--contact".into(),
                    s(&dir.join("contact.png")),
                ],
                out: None,
            });
            did.push(format!("fitted again from the {kind} in {}", src.display()));
            // Retry prompts for the marked slots' cells, when the sheets are
            // the tab's own (they are written into the sheets folder).
            if kind == "sheets" && src.starts_with(work(at)) {
                let cells = marked_cells(src, &marks);
                if !cells.is_empty() {
                    steps.push(Step {
                        args: vec![
                            "theme".into(),
                            "sheet".into(),
                            s(at.dir),
                            "-o".into(),
                            s(src),
                            "--retry".into(),
                            cells.join(","),
                        ],
                        out: None,
                    });
                    did.push(format!("retry prompts for cells {}", cells.join(", ")));
                }
            }
            out
        }
        None => {
            did.push("checked as it is: the tab doesn't know the art it was fitted from".into());
            base.dir.clone()
        }
    };
    steps.push(Step {
        args: vec![
            "--json".into(),
            "theme".into(),
            "check".into(),
            s(at.dir),
            "--theme".into(),
            s(&theme_dir),
            "--original".into(),
        ],
        out: Some(dir.join("check.json")),
    });
    let record = dir.join("round.json");
    let roles: BTreeMap<String, String> = kit(at)["art"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["id"].as_str()?.to_owned(), a["role"].as_str()?.to_owned())))
        .collect();
    let (key, marks2, did2) = (base_key.clone(), marks.clone(), did.clone());
    let started = now();
    let then: Then = Box::new(move |exits: &[i32]| {
        let check: Value = fs::read_to_string(record.with_file_name("check.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
        let refused = check["originality"]["scores"].as_array().map_or(0, |a| {
            a.iter().filter(|s| s["refused"] == json!(true)).count()
        });
        let v = json!({
            "n": n, "theme": key, "theme_dir": theme_dir, "did": did2,
            "exits": exits, "started": started, "ended": now(),
            "readability": check["readability"]["verdict"],
            "originality_refused": refused,
            "geometry_refused": check["geometry"]["flags"].as_array().map_or(0, |a| a.iter().filter(|f| f["verdict"] == json!("refuse")).count()),
            "marks": judge_marks(&marks2, &check, &roles),
        });
        let _ = write_atomic(
            &record,
            &serde_json::to_string_pretty(&v).unwrap_or_default(),
        );
    });
    let job = spawn_job(at, "round", Some(&base_key), steps, true, Some(then))?;
    Ok(Response::json(&json!({"job": job, "round": n, "did": did})))
}

/// The cells of a sheets folder's grids that hold the marked slots, from
/// its sheets.json.
fn marked_cells(sheets: &Path, marks: &[Value]) -> Vec<String> {
    let Ok(m) = crate::theme_sheet::Manifest::load(sheets) else {
        return Vec::new();
    };
    let slots: Vec<&str> = marks.iter().filter_map(|m| m["slot"].as_str()).collect();
    let mut out: Vec<String> = m
        .sheets
        .iter()
        .flat_map(|s| s.cells.iter())
        .filter(|c| slots.contains(&c.slot.as_str()))
        .map(|c| c.cell.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marks_state_follows_its_slots_checks() {
        let marks = vec![
            json!({"id": 1, "slot": "entity.player", "note": "muddy"}),
            json!({"id": 2, "slot": "entity.barrel", "note": "too like it"}),
            json!({"id": 3, "slot": "entity.hammer", "note": "head off"}),
            json!({"id": 4, "slot": "entity.fire", "note": "fine now"}),
            json!({"id": 5, "slot": null, "note": "somewhere"}),
        ];
        let check = json!({
            "readability": {"groups": [{"group": "player", "verdict": "refuse"}, {"group": "enemies", "verdict": "ok"}]},
            "originality": {"scores": [
                {"what": "entity.barrel", "value": 0.9, "refused": true},
                {"what": "entity.fire", "value": 0.2, "refused": false},
            ]},
            "geometry": {"flags": [{"slot": "entity.hammer", "verdict": "refuse"}]},
        });
        let roles: BTreeMap<String, String> = [
            ("entity.player", "player"),
            ("entity.barrel", "enemy"),
            ("entity.hammer", "power-up"),
            ("entity.fire", "enemy"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .collect();
        let j = judge_marks(&marks, &check, &roles);
        let states: Vec<&str> = j.iter().map(|m| m["state"].as_str().unwrap()).collect();
        assert_eq!(
            states,
            [
                "still hard to read: the readability check refuses its group",
                "still too close to the original",
                "still refused by the geometry check",
                "passes its checks",
                "no slot named: see the checks",
            ]
        );
        assert_eq!(j[3]["originality"], json!(0.2));
    }

    #[test]
    fn names_from_the_page_stay_in_their_folder() {
        assert!(safe("dk.m50-ladders.005"));
        assert!(safe("e9cb1d89ad40"));
        for bad in ["", "..", "../x", "a/b", ".hidden", "a b"] {
            assert!(!safe(bad), "{bad}");
        }
        assert_eq!(group_of("hazard"), "hazards");
        assert_eq!(group_of("npc"), "");
    }
}
