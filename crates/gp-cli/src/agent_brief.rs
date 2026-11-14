//! `gp agent brief` and `gp agent clips` (game-printer's agent_brief.rs;
//! docs/agents.md, "The brief"): everything an agent needs to start on a
//! clip, in one call. The clip and where it starts, its score and first
//! wrong frame, each wrong measure's values around that frame, the
//! mechanics that own those measures with their parameters as the model
//! sets them, the code the model's evidence cites for them (file, line and
//! routine), what the original's code read to write the first wrong
//! measure (`gp element inputs`: the 6502 writer slice on an NES game, the
//! Z80 one on an arcade game), the person's open tells, the attempts so
//! far, and the commands to run next, written out in full.
//!
//! It only reads. A run is a game of the games checkout, as in `gp serve`
//! and the rest of `gp agent`: its clips are the background work's active
//! clips and their scores (`$GP_HOME/work/serve/GAME/background/`), its
//! model the run's working copy once there is one. game-printer's brief
//! read a capture run; the values around the first wrong frame came from
//! the queue's curve, which gameprinter's queue doesn't keep, so the brief
//! plays the clip's lockstep once itself.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::serve::Ctx;
use crate::serve::background;
use crate::serve::game::{Run, run_named};
use crate::serve::work;

/// The brief's JSON shape. A change to it bumps this.
pub const BRIEF_SCHEMA: &str = "gp agent brief 3";

/// Frames of each wrong measure shown before and after its first wrong
/// frame.
const BEFORE: i64 = 3;
const AFTER: i64 = 5;
/// Caps, so a brief stays a few thousand tokens. Each says how to see more.
const MAX_FACTS: usize = 12;
const MAX_REFS: usize = 6;
const MAX_NOTE: usize = 400;
const MAX_MEASURES: usize = 6;

pub struct BriefArgs<'a> {
    pub run: &'a str,
    pub clip: &'a str,
    /// Play the clip's lockstep to give each wrong measure's values around
    /// its first wrong frame, and slice the first one's writer.
    pub curve: bool,
}

fn run_of(ctx: &Ctx, run: &str) -> anyhow::Result<Run> {
    run_named(ctx, run).map_err(|f| anyhow::anyhow!(f.message))
}

/// A clip's score as the background work keeps it, if it has one.
fn score_of<'a>(state: &'a Value, id: &str) -> Option<&'a Value> {
    state["scores"].get(id).filter(|s| s.is_object())
}

fn percent(s: &Value) -> u64 {
    work::pct(s["permille"].as_u64().unwrap_or(0))
}

pub fn run(ctx: &Ctx, paths: &Paths, a: &BriefArgs) -> anyhow::Result<Outcome> {
    let v = brief(ctx, paths, a)?;
    let text = text_of(&v);
    Ok(Outcome::ok(v, text))
}

/// `gp agent clips`: every active clip of the run with its score, those
/// that don't match first.
pub fn clips(ctx: &Ctx, run: &str) -> anyhow::Result<Outcome> {
    let r = run_of(ctx, run)?;
    let all = background::active_now(ctx, &r);
    let state = background::saved_state(&r);
    let mut v = run_clips(ctx, &r, &all, &state, "", usize::MAX);
    v["schema"] = json!("gp agent clips 1");
    v["run"] = json!(run);
    let mut text = format!(
        "{} active clips in {run}, {} match.",
        v["total"], v["matching"]
    );
    for c in v["list"].as_array().into_iter().flatten() {
        text.push_str(&format!(
            "\n- {} ({}), {} frames, {}{}",
            c["id"].as_str().unwrap_or(""),
            c["name"].as_str().unwrap_or(""),
            c["frames"],
            if c["exact"] == true {
                "matches".to_owned()
            } else if c["percent"].is_null() {
                "not scored yet".to_owned()
            } else {
                format!("{}%", c["percent"])
            },
            c["first_wrong"]
                .as_u64()
                .map_or(String::new(), |f| format!(", first wrong at frame {f}"))
        ));
    }
    Ok(Outcome::ok(v, text))
}

/// The brief as JSON ([`BRIEF_SCHEMA`]).
pub fn brief(ctx: &Ctx, paths: &Paths, a: &BriefArgs) -> anyhow::Result<Value> {
    let r = run_of(ctx, a.run)?;
    let state = background::saved_state(&r);
    let clips = background::active_now(ctx, &r);
    let Some(clip) = clips.iter().find(|c| c.id == a.clip) else {
        let near: Vec<&str> = clips
            .iter()
            .map(|c| c.id.as_str())
            .filter(|id| score_of(&state, id).is_some_and(|s| s["exact"] != true))
            .take(8)
            .collect();
        anyhow::bail!(
            "no active clip {} in run {}. Clips that don't match yet: {}. `gp agent clips --run {}` lists every clip",
            a.clip,
            a.run,
            if near.is_empty() {
                "none".to_owned()
            } else {
                near.join(", ")
            },
            a.run
        );
    };
    let gp = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gp"));
    let gp = gp.display().to_string();
    let score = score_of(&state, &clip.id);
    let exact = score.is_some_and(|s| s["exact"] == true);
    let model_dir = r.model_dir();
    let module = r.rebuild(ctx).module;
    let model_text = fs::read_to_string(model_dir.join(gp_model::MODEL_FILE)).unwrap_or_default();
    let tree: toml::Value = toml::from_str(&model_text).unwrap_or(toml::Value::Integer(0));
    let platform = r.platform();

    // The clip's input, its start and its frames.
    let input = crate::serve::scenarios::clip_input(ctx, &r, &clip.id).ok();
    let (start, frames) = input
        .as_ref()
        .map_or((None, None), |(_, at, n, _)| (Some(*at), Some(*n)));

    // The clip's lockstep, once, for the wrong measures and the values
    // around them; else the measure the background score names.
    let played = if a.curve && !exact {
        Some(play(
            paths,
            &r,
            &model_dir,
            module.as_deref(),
            input.as_ref(),
        ))
    } else {
        None
    };
    let wrong: Vec<Wrong> = match &played {
        Some(Ok((tracks, _))) => tracks.clone(),
        _ => score.map(wrong_of).unwrap_or_default(),
    };
    let curve = played.as_ref().map(|p| match p {
        Ok((_, c)) => c.clone(),
        Err(e) => json!({"error": format!("{e:#}")}),
    });
    let owners = owners(&model_text, &model_dir, &tree, &wrong);
    let code = citations(ctx, &paths.home, &tree, &owners, &wrong);
    let questions: Vec<Value> = background::open_questions(&r)
        .into_iter()
        .filter(|q| q["clip"].as_str().is_none_or(|c| c == clip.id))
        .collect();
    let agent = agent_status(&r, &clip.id);
    let group: Vec<String> = agent
        .as_ref()
        .and_then(|s| s["clips"].as_array())
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.iter()
                .filter_map(|c| c.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_else(|| vec![clip.id.clone()]);
    let attempts: Vec<Value> = work::attempts(&r)
        .into_iter()
        .filter(|x| x.clip == clip.id || group.contains(&x.clip))
        .map(|x| {
            let pct = |s: &Option<work::Side>| s.as_ref().map(|s| work::pct(s.permille));
            json!({"n": x.n, "by": x.by, "clip": x.clip, "title": x.title,
                   "before": pct(&x.before), "after": pct(&x.after),
                   "exact_after": x.after.as_ref().is_some_and(|s| s.exact),
                   "first_wrong_after": x.after.as_ref().and_then(|s| s.first_wrong),
                   "better": x.better.len(), "worse": x.worse.len(),
                   "kept": x.approved || x.keepable(), "commit": x.commit,
                   "dir": work::folder(&r).join(x.n.to_string()),
                   "line": work::attempt_line(&x)})
        })
        .collect();
    // The writer slice of the first wrong measure (`gp element inputs`):
    // the routine and every input it read on that frame, on the main Z80
    // for an arcade game.
    let inputs = if a.curve && !exact && !wrong.is_empty() {
        let first = wrong.first();
        let ia = crate::element::InputsArgs {
            run: r.game.clone(),
            clip: clip.id.clone(),
            measure: first.map(|w| w.path.clone()),
            frame: first.map(|w| w.frame).filter(|f| *f >= 1),
            model: Some(model_dir.clone()),
            module: module.clone(),
            depth: 1,
            back: true,
            ..crate::element::InputsArgs::default()
        };
        Some(match crate::element::inputs(paths, &ia) {
            Ok(v) => v,
            Err(e) => json!({"error": format!("{e:#}")}),
        })
    } else {
        None
    };
    let worktree = agent
        .as_ref()
        .and_then(|s| s["worktree"].as_str().map(str::to_owned));
    let tells = crate::serve::tell::brief(&r, &group);
    let clip_list = run_clips(ctx, &r, &clips, &state, &clip.id, MAX_CLIPS);
    let first_ref = code["facts"].as_array().and_then(|f| {
        f.iter()
            .flat_map(|x| x["refs"].as_array().into_iter().flatten())
            .find(|r| r["there"] == true)
            .cloned()
    });
    let next = next_commands(&Next {
        gp: &gp,
        games: &ctx.games,
        run: a.run,
        game: &r.game,
        clip: &clip.id,
        model: &model_dir,
        module: module.as_deref(),
        worktree: worktree.as_deref(),
        first_ref: first_ref.as_ref(),
        owners: &owners,
        exact,
        arcade: platform == crate::roms::Platform::Arcade,
    });
    Ok(json!({
        "schema": BRIEF_SCHEMA,
        "run": a.run,
        "run_dir": r.dir,
        "game": r.game,
        "platform": if platform == crate::roms::Platform::Arcade { "arcade" } else { "nes" },
        "model": model_dir,
        "module": module,
        "gp": gp,
        "clip": {
            "id": clip.id, "name": clip.name, "frames": frames, "starred": clip.starred,
            "start": start.map(|s| json!({"frame": s})),
        },
        "score": score.map(|s| json!({
            "percent": percent(s), "exact": s["exact"] == true,
            "first_wrong": s["drift"], "first_path": s["drift_path"],
            "says": s["says"], "error": s["error"], "model": s["model"],
        })),
        "measures": wrong.iter().map(|w| json!({
            "track": w.track, "path": w.path, "frame": w.frame,
            "original": w.original, "rebuilt": w.rebuilt, "says": w.says,
        })).collect::<Vec<_>>(),
        "curve": curve,
        "inputs": inputs,
        "owners": owners,
        "code": code,
        "disassembly": disassemblies(&paths.home, &model_text),
        "clips": clip_list,
        "files": {
            "run": r.dir,
            "agent": work::folder(&r),
            "notes": work::folder(&r).join("work.jsonl"),
            "attempts": "agent/<n>/ under `agent`; `attempts` below names each one's folder",
        },
        "questions": questions,
        "tells": tells,
        "group": group,
        "attempts": attempts,
        "agent": agent.as_ref().map(|s| json!({
            "state": s["state"], "worktree": s["worktree"], "branch": s["branch"],
        })),
        "next": next,
    }))
}

/// The clip's lockstep straight through on the run's model: its wrong
/// measures (each track's first) and each one's values around its first
/// wrong frame.
fn play(
    paths: &Paths,
    r: &Run,
    model_dir: &Path,
    module: Option<&Path>,
    input: Option<&(gp_evidence::InputLog, usize, u64, Value)>,
) -> anyhow::Result<(Vec<Wrong>, Value)> {
    let (log, start, _, _) =
        input.ok_or_else(|| anyhow::anyhow!("the clip's input can't be read"))?;
    let sc = crate::clip_score::Ctx::open(
        paths,
        &r.game,
        &crate::clip_score::Open {
            module,
            model: Some(model_dir),
            assets: None,
            rom: None,
        },
    )?;
    let (ls, _, _) = crate::clip_score::straight(&sc, log, *start, None, true)?;
    let tracks = gp_interp::scenario::tracks(&ls, *start as u64);
    let wrong: Vec<Wrong> = tracks
        .iter()
        .map(|t| {
            let (kind, field) = kind_field(&t.path);
            Wrong {
                track: t.track.clone(),
                path: t.path.clone(),
                kind,
                field,
                frame: t.frame,
                original: t.original.clone(),
                rebuilt: t.model.clone(),
                says: t.says.clone(),
            }
        })
        .collect();
    Ok((wrong.clone(), window(&ls, *start as u64, &wrong)))
}

/// The pinned disassemblies the model cites (`in = "NAME"`): where each
/// file is, and whether it is downloaded. gameprinter pins none yet
/// (pinned.rs), so this is empty until one is.
fn disassemblies(home: &Path, model_text: &str) -> Vec<Value> {
    crate::pinned::PINS
        .iter()
        .filter(|p| model_text.contains(&format!("\"{}\"", p.name)))
        .map(|p| {
            let root = home.join("work").join(p.dir);
            json!({
                "name": p.name,
                "folder": root,
                "files": p.files.iter().map(|(f, _)| root.join(f)).collect::<Vec<_>>(),
                "there": p.files.iter().all(|(f, _)| root.join(f).is_file()),
                "fetch": p.fetch,
            })
        })
        .collect()
}

/// How many of the run's clips the brief lists.
const MAX_CLIPS: usize = 20;

/// The run's active clips, those that don't match first, so an agent
/// needn't read the background work's state.
fn run_clips(
    ctx: &Ctx,
    r: &Run,
    clips: &[background::Active],
    state: &Value,
    mine: &str,
    limit: usize,
) -> Value {
    // Each clip's frames: a corpus clip's from its recipe, a free play
    // clip's from its session.
    let mut frames_of: BTreeMap<String, u64> = crate::serve::generated::made(r)
        .into_iter()
        .map(|m| (m.id, m.frames))
        .collect();
    for c in crate::serve::freeplay::clips_for_run(&ctx.paths.home, &r.name) {
        frames_of.insert(c.id, c.frames);
    }
    let mut rows: Vec<(bool, Value)> = clips
        .iter()
        .map(|c| {
            let s = score_of(state, &c.id);
            let exact = s.is_some_and(|s| s["exact"] == true);
            let frames = frames_of.get(&c.id);
            (
                exact,
                json!({"id": c.id, "name": c.name, "frames": frames,
                       "percent": s.map(percent), "exact": exact,
                       "first_wrong": s.and_then(|s| s["drift"].as_u64()), "mine": c.id == mine}),
            )
        })
        .collect();
    rows.sort_by_key(|(exact, v)| (*exact, v["mine"] != true));
    let total = rows.len();
    let matching = rows.iter().filter(|(e, _)| *e).count();
    let shown: Vec<Value> = rows.into_iter().take(limit).map(|(_, v)| v).collect();
    json!({"total": total, "matching": matching,
           "more": total.saturating_sub(shown.len()), "list": shown,
           "all": "gp agent clips --run <run>"})
}

/// A measure wrong at its track's first wrong frame.
#[derive(Clone)]
struct Wrong {
    track: String,
    path: String,
    /// The entity kind (`mario`, `barrel`), from the path.
    kind: String,
    /// The field measured (`y`, `facing`).
    field: String,
    frame: u64,
    original: String,
    rebuilt: String,
    says: String,
}

/// The wrong measures a background score names (its first only).
fn wrong_of(s: &Value) -> Vec<Wrong> {
    s["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let path = t["path"].as_str().unwrap_or("").to_owned();
            let (kind, field) = kind_field(&path);
            Wrong {
                track: t["track"].as_str().unwrap_or("").to_owned(),
                kind,
                field,
                frame: t["frame"].as_u64().unwrap_or(0),
                original: t["original"].as_str().unwrap_or("").to_owned(),
                rebuilt: t["model"].as_str().unwrap_or("").to_owned(),
                says: t["says"].as_str().unwrap_or("").to_owned(),
                path,
            }
        })
        .collect()
}

/// `bodies.spiny[0].y` -> (`spiny`, `y`); `entities.mario[0].facing` ->
/// (`mario`, `facing`); `camera.x` -> (`camera`, `x`).
fn kind_field(path: &str) -> (String, String) {
    let parts: Vec<&str> = path.split('.').collect();
    let bare = |s: &str| s.split('[').next().unwrap_or(s).to_owned();
    match parts.as_slice() {
        [root, kind, rest @ ..] if matches!(*root, "entities" | "bodies") && !rest.is_empty() => {
            (bare(kind), rest.join("."))
        }
        [first, rest @ ..] if !rest.is_empty() => (bare(first), rest.join(".")),
        _ => (path.to_owned(), String::new()),
    }
}

/// The mechanics on the wrong measures' entities: the ones that write the
/// measured field own it, the others are on the same entity. Each with its
/// parameters as the run's working copy sets them, and where.
fn owners(model_text: &str, model_dir: &Path, tree: &toml::Value, wrong: &[Wrong]) -> Vec<Value> {
    let Ok(m) = gp_model::Model::parse(model_text) else {
        return Vec::new();
    };
    // The hooks the model's mechanics name are in its game's module; the
    // catalog reads their records from the hooks gp itself has, and names
    // the others as opaque.
    let (exts, _) = gp_exts::registry(&m.model.name);
    let cat = gp_interp::mechanics::catalog(&m, &exts);
    let model_file = model_dir.join(gp_model::MODEL_FILE);
    let mut out: Vec<(bool, Value)> = Vec::new();
    for mm in &cat.mechanics {
        for at in &mm.used_by {
            let Some(kind) = at
                .strip_prefix("entities.")
                .and_then(|r| r.split('.').next())
            else {
                continue;
            };
            let on: Vec<&Wrong> = wrong.iter().filter(|w| w.kind == kind).collect();
            if on.is_empty() {
                continue;
            }
            let writes: Vec<&str> = on
                .iter()
                .filter(|w| {
                    let f = w.field.split('.').next().unwrap_or("");
                    mm.writes.iter().any(|x| x.strip_prefix("self.") == Some(f))
                })
                .map(|w| w.path.as_str())
                .collect();
            let table = get_path(tree, at);
            let params: BTreeMap<String, String> = mm
                .params
                .iter()
                .filter_map(|p| {
                    let v = table.and_then(|t| t.get(p.name))?;
                    Some((p.name.to_owned(), short_toml(v)))
                })
                .collect();
            let line = header_line(model_text, at);
            out.push((
                !writes.is_empty(),
                json!({
                    "mechanic": mm.name, "version": mm.version, "at": at,
                    "owns": writes, "about": mm.about, "params": params,
                    "unset": mm.params.iter().map(|p| p.name).filter(|n| !params.contains_key(*n)).collect::<Vec<_>>(),
                    "file": line.map(|l| format!("{}:{l}", model_file.display())),
                }),
            ));
        }
    }
    // Owners first, then the rest, each in catalog order.
    out.sort_by_key(|(owns, _)| !*owns);
    out.into_iter().map(|(_, v)| v).collect()
}

fn get_path<'a>(tree: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    path.split('.').try_fold(tree, |t, k| t.get(k))
}

/// A parameter's value in one line, cut to 120 characters.
fn short_toml(v: &toml::Value) -> String {
    let s = match v {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    cut(&s.replace('\n', " "), 120)
}

/// The line of `[path]` in the model, 1-based.
fn header_line(text: &str, path: &str) -> Option<usize> {
    let want = format!("[{path}]");
    text.lines().position(|l| l.trim() == want).map(|i| i + 1)
}

fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

/// The code the model's evidence cites for the owners and the wrong
/// entities: each fact's note, cut, and its refs as absolute file and line
/// with the routine the line is in, for refs into a pinned download (the
/// disassembly) or the repo.
fn citations(
    ctx: &Ctx,
    home: &Path,
    tree: &toml::Value,
    owners: &[Value],
    wrong: &[Wrong],
) -> Value {
    let Some(ev) = tree.get("evidence").and_then(|e| e.as_table()) else {
        return json!({"facts": [], "more": 0});
    };
    let mut prefixes: Vec<String> = owners
        .iter()
        .filter(|o| o["owns"].as_array().is_some_and(|a| !a.is_empty()))
        .filter_map(|o| o["at"].as_str().map(str::to_owned))
        .collect();
    for w in wrong {
        prefixes.push(format!("entities.{}", w.kind));
    }
    prefixes.sort();
    prefixes.dedup();
    let repo = ctx.repo.clone();
    let mut resolver = crate::pinned::Resolver::new(home);
    let mut files: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    // Facts owned by an owner's path first, then the entity's own.
    let mut keys: Vec<(&String, &toml::Value)> = ev
        .iter()
        .filter(|(k, _)| {
            prefixes
                .iter()
                .any(|p| *k == p || k.starts_with(&format!("{p}.")))
        })
        .collect();
    keys.sort_by_key(|(k, _)| {
        let owned = owners
            .iter()
            .filter(|o| o["owns"].as_array().is_some_and(|a| !a.is_empty()))
            .any(|o| o["at"].as_str().is_some_and(|at| k.starts_with(at)));
        (!owned, k.len())
    });
    let total = keys.len();
    let mut facts = Vec::new();
    for (k, v) in keys.into_iter().take(MAX_FACTS) {
        let mut refs = Vec::new();
        let all = v
            .get("refs")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        for r in all.iter().take(MAX_REFS) {
            let Some(at) = r.get("at").and_then(|x| x.as_str()) else {
                continue;
            };
            let within = r.get("in").and_then(|x| x.as_str());
            let Some((file, line)) = at
                .rsplit_once(':')
                .and_then(|(f, l)| Some((f, l.parse::<usize>().ok()?)))
            else {
                continue;
            };
            let path = match resolver.root(within, file, &repo) {
                gp_model::validate::RefRoot::Dir(d) => d.join(file),
                _ => continue,
            };
            let lines = files.entry(path.clone()).or_insert_with(|| {
                fs::read_to_string(&path)
                    .map(|t| t.lines().map(str::to_owned).collect())
                    .unwrap_or_default()
            });
            let routine = routine_at(lines, line);
            refs.push(json!({
                "file": path, "line": line, "routine": routine,
                "quote": r.get("quote").and_then(|x| x.as_str()),
                "there": !lines.is_empty(),
            }));
        }
        facts.push(json!({
            "path": k,
            "source": v.get("source").and_then(|x| x.as_str()),
            "protected": v.get("source").and_then(|x| x.as_str()) == Some("stated"),
            "note": v.get("note").and_then(|x| x.as_str()).map(|n| cut(n, MAX_NOTE)),
            "refs": refs,
            "more_refs": all.len().saturating_sub(MAX_REFS),
        }));
    }
    json!({"facts": facts, "more": total.saturating_sub(MAX_FACTS),
           "missing": resolver.missing.iter().map(|(n, f)| json!({"download": n, "fetch": f})).collect::<Vec<_>>()})
}

/// The label a line of assembly is under: the nearest line at or above it
/// that starts with a name and a colon (`JumpEngine:`, `ImposeGravity:`).
fn routine_at(lines: &[String], line: usize) -> Option<String> {
    let end = line.min(lines.len());
    lines[..end].iter().rev().find_map(|l| {
        let (name, _) = l.split_once(':')?;
        let ok = !name.is_empty()
            && name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        ok.then(|| name.to_owned())
    })
}

/// Each wrong measure's original and rebuilt values from a few frames
/// before its first wrong frame to a few after, from the lockstep's values
/// (`start` is the clip's start in the original's frames).
fn window(ls: &gp_interp::lockstep::Lockstep, start: u64, wrong: &[Wrong]) -> Value {
    let mut paths: Vec<&Wrong> = Vec::new();
    for w in wrong {
        if !paths.iter().any(|p| p.path == w.path) {
            paths.push(w);
        }
    }
    let mut out = Vec::new();
    for w in paths.iter().take(MAX_MEASURES) {
        let Some(k) = ls.paths.iter().position(|p| p.path == w.path) else {
            continue;
        };
        let first = w.frame as i64;
        let rows: Vec<Value> = ls
            .frames
            .iter()
            .zip(&ls.values)
            .filter_map(|((_, orig), (o, m))| {
                let f = orig.checked_sub(start)? as i64;
                ((first - BEFORE..=first + AFTER).contains(&f)).then(|| {
                    json!({"frame": f,
                           "original": o.get(k).map(value_text),
                           "rebuilt": m.get(k).map(value_text)})
                })
            })
            .collect();
        out.push(json!({"path": w.path, "words": w.says, "first_wrong": first, "frames": rows}));
    }
    json!({"measures": out, "full": "the whole curve: gp clip score GAME CLIP --values PATH"})
}

/// A measured value as the score's words give it: a fixed-point number
/// in units, not 1/65536ths.
fn value_text(v: &gp_runtime::Value) -> String {
    match v {
        gp_runtime::Value::Fixed(x) => format!("{}", *x as f64 / 65536.0),
        gp_runtime::Value::Text(t) => t.clone(),
        other => other.to_string(),
    }
}

/// The clip agent's status for the clip, or for the group holding it.
fn agent_status(r: &Run, clip: &str) -> Option<Value> {
    let folder = work::folder(r).join("clip-agents");
    fs::read_dir(folder).ok()?.flatten().find_map(|e| {
        let v: Value = serde_json::from_slice(&fs::read(e.path()).ok()?).ok()?;
        let hit = v["clip"] == clip
            || v["clips"]
                .as_array()
                .is_some_and(|a| a.iter().any(|c| c == clip));
        hit.then_some(v)
    })
}

struct Next<'a> {
    gp: &'a str,
    games: &'a Path,
    run: &'a str,
    game: &'a Path,
    clip: &'a str,
    model: &'a Path,
    module: Option<&'a Path>,
    worktree: Option<&'a str>,
    first_ref: Option<&'a Value>,
    owners: &'a [Value],
    exact: bool,
    arcade: bool,
}

/// Quote an argument for a POSIX shell when it needs it, in double
/// quotes as the clip agent's prompt writes them.
fn q(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-:@=+,".contains(c))
    {
        s.to_owned()
    } else {
        let mut o = String::from("\"");
        for c in s.chars() {
            if matches!(c, '"' | '\\' | '$' | '`') {
                o.push('\\');
            }
            o.push(c);
        }
        o.push('"');
        o
    }
}

/// The commands to run next, in order, each whole: deterministic tools
/// first, then reading the code, then one change and its attempt.
fn next_commands(n: &Next) -> Vec<Value> {
    let gp = q(n.gp);
    let agent = format!("{gp} agent --games {}", q(&n.games.display().to_string()));
    let (run, clip) = (q(n.run), q(n.clip));
    let game = q(&n.game.display().to_string());
    let model = q(&n.model.display().to_string());
    let module = n.module.map_or(String::new(), |m| {
        format!(" --module {}", q(&m.display().to_string()))
    });
    let mut out = Vec::new();
    let mut add = |why: &str, cmd: String| out.push(json!({"why": why, "run": cmd}));
    if n.exact {
        add(
            "the clip matches; say so and stop",
            format!(
                "{agent} done --run {run} --clip {clip} --by \"clip agent\" \"the clip matches\""
            ),
        );
        return out;
    }
    add(
        "say what you are on, for the person watching",
        format!(
            "{agent} note --run {run} --clip {clip} --by \"clip agent\" --on \"<a few words>\" \"<what you are doing>\""
        ),
    );
    add(
        "the deterministic solvers first: a number they can fix needs no code change",
        format!("{gp} --json solve {game} --run {run}{module}"),
    );
    add(
        if n.arcade {
            "what the original's main Z80 read to write the first wrong measure"
        } else {
            "what the original's code read to write the first wrong measure"
        },
        format!("{gp} element inputs --run {game} --clip {clip} --model {model}{module}"),
    );
    if let Some(r) = n.first_ref {
        let line = r["line"].as_u64().unwrap_or(1);
        add(
            "the code the owner's evidence cites (the routine it is in)",
            format!(
                "sed -n {},{}p {}",
                line.saturating_sub(10).max(1),
                line + 30,
                q(r["file"].as_str().unwrap_or(""))
            ),
        );
    }
    if let Some(o) = n.owners.first().and_then(|o| o["file"].as_str()) {
        add(
            "the owner's parameters in the run's model (edit your copy of it, not this)",
            format!(
                "sed -n {}p {}",
                o.rsplit_once(':')
                    .map_or("1,40".to_owned(), |(_, l)| format!(
                        "{l},{}",
                        l.parse::<usize>().unwrap_or(1) + 40
                    )),
                q(o.rsplit_once(':').map_or(o, |(f, _)| f))
            ),
        );
    }
    add(
        "this clip's score and first wrong measure on the run's model, as JSON",
        format!(
            "{gp} --json {} score --run {run} --model {model} --clip {clip}",
            agent.trim_start_matches(&format!("{gp} "))
        ),
    );
    let wt = n.worktree.map_or("<your worktree>".to_owned(), q);
    add(
        "after one change in the worktree: commit, build, score every active clip, preview",
        format!(
            "{agent} attempt --run {run} --clip {clip} --by \"clip agent\" --worktree {wt} --model-from <your copy of the model> --commit --title \"<the change in a few words>\" --says \"<what changed and why>\""
        ),
    );
    add(
        "when you stop",
        format!("{agent} done --run {run} --clip {clip} --by \"clip agent\" \"<why>\""),
    );
    out
}

/// The brief in plain text, for a person or an agent that reads text.
fn text_of(v: &Value) -> String {
    let mut o = String::new();
    let mut line = |s: String| {
        o.push_str(&s);
        o.push('\n');
    };
    let c = &v["clip"];
    line(format!(
        "Clip {} ({}), {} frames, run {}.",
        c["id"].as_str().unwrap_or(""),
        c["name"].as_str().unwrap_or(""),
        c["frames"],
        v["run"].as_str().unwrap_or("")
    ));
    match v["score"].as_object() {
        Some(s) if s["exact"] == true => line("It matches.".into()),
        Some(s) => line(format!(
            "{}% together; first wrong at frame {}: {}.",
            s["percent"],
            s["first_wrong"],
            s["first_path"].as_str().unwrap_or("?")
        )),
        None => line("Not scored yet.".into()),
    }
    for m in v["measures"].as_array().into_iter().flatten() {
        line(format!("- {}", m["says"].as_str().unwrap_or("")));
    }
    for m in v["curve"]["measures"].as_array().into_iter().flatten() {
        let rows: Vec<String> = m["frames"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| format!("{}: {} / {}", r["frame"], r["original"], r["rebuilt"]))
            .collect();
        line(format!(
            "  {} (original / rebuilt): {}",
            m["path"].as_str().unwrap_or(""),
            rows.join(", ")
        ));
    }
    if let Some(e) = v["curve"]["error"].as_str() {
        line(format!("  No curve: {e}"));
    }
    if let Some(e) = v["inputs"]["error"].as_str() {
        line(format!("No writer slice: {e}"));
    } else if v["inputs"]["bytes"].is_array() {
        line(format!(
            "What the original's code read (gp element inputs --run {} --clip {}):",
            v["game"].as_str().unwrap_or(""),
            v["clip"]["id"].as_str().unwrap_or("")
        ));
        for l in crate::element::text_of(&v["inputs"]).lines() {
            line(format!("  {l}"));
        }
    }
    for t in v["tells"].as_array().into_iter().flatten() {
        line(format!(
            "The person told you (reply with gp agent note --reply-to N): {}",
            t.as_str().unwrap_or("")
        ));
    }
    for x in v["questions"].as_array().into_iter().flatten() {
        line(format!(
            "Open question: {}",
            x["ask"]
                .as_str()
                .or(x["text"].as_str())
                .or(x["missing"].as_str())
                .or(x["detail"].as_str())
                .unwrap_or("")
        ));
    }
    line("Owners (the mechanics that write a wrong measure):".into());
    let all = v["owners"].as_array().cloned().unwrap_or_default();
    let (own, rest): (Vec<&Value>, Vec<&Value>) = all
        .iter()
        .partition(|o| o["owns"].as_array().is_some_and(|a| !a.is_empty()));
    for ow in own {
        let params: Vec<String> = ow["params"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, x)| format!("{k}={}", x.as_str().unwrap_or("")))
            .collect();
        let owns: Vec<&str> = ow["owns"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        line(format!(
            "- {} {} at {} writes {}{}{}",
            ow["mechanic"].as_str().unwrap_or(""),
            ow["version"].as_str().unwrap_or(""),
            ow["at"].as_str().unwrap_or(""),
            owns.join(", "),
            ow["file"]
                .as_str()
                .map_or(String::new(), |f| format!(", {f}")),
            if params.is_empty() {
                String::new()
            } else {
                format!(": {}", params.join(", "))
            }
        ));
    }
    if !rest.is_empty() {
        let names: Vec<String> = rest
            .iter()
            .map(|o| {
                format!(
                    "{} at {}",
                    o["mechanic"].as_str().unwrap_or(""),
                    o["at"].as_str().unwrap_or("")
                )
            })
            .collect();
        line(format!(
            "Also on those entities: {} (their parameters are in --json).",
            names.join(", ")
        ));
    }
    line("Code the evidence cites:".into());
    for f in v["code"]["facts"].as_array().into_iter().flatten() {
        let refs: Vec<String> = f["refs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| {
                format!(
                    "{}:{}{}",
                    r["file"].as_str().unwrap_or(""),
                    r["line"],
                    r["routine"]
                        .as_str()
                        .map_or(String::new(), |x| format!(" ({x})"))
                )
            })
            .collect();
        line(format!(
            "- {}{}: {}",
            f["path"].as_str().unwrap_or(""),
            if f["protected"] == true {
                " (protected)"
            } else {
                ""
            },
            if refs.is_empty() {
                "no code refs".to_owned()
            } else {
                refs.join("; ")
            }
        ));
    }
    if v["code"]["more"].as_u64().unwrap_or(0) > 0 {
        line(format!(
            "  ({} more facts; --json has the same list)",
            v["code"]["more"]
        ));
    }
    for m in v["code"]["missing"].as_array().into_iter().flatten() {
        line(format!(
            "  The download {} isn't there; fetch it with `{}`.",
            m["download"].as_str().unwrap_or(""),
            m["fetch"].as_str().unwrap_or("")
        ));
    }
    let cl = &v["clips"];
    line(format!(
        "The run's clips: {} active, {} match. Those that don't ({} listed; `{} --run {}` lists all):",
        cl["total"],
        cl["matching"],
        cl["list"].as_array().map_or(0, Vec::len),
        cl["all"]
            .as_str()
            .unwrap_or("gp agent clips")
            .replace(" --run <run>", ""),
        v["run"].as_str().unwrap_or("")
    ));
    for c in cl["list"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["exact"] != true)
    {
        line(format!(
            "- {} ({}), {}{}{}",
            c["id"].as_str().unwrap_or(""),
            c["name"].as_str().unwrap_or(""),
            c["percent"]
                .as_u64()
                .map_or("not scored yet".to_owned(), |p| format!("{p}%")),
            c["first_wrong"]
                .as_u64()
                .map_or(String::new(), |f| format!(", first wrong at frame {f}")),
            if c["mine"] == true {
                " (this clip)"
            } else {
                ""
            }
        ));
    }
    line(format!(
        "Run files: {} (notes: {}). Each attempt is a folder under {}; the lines below name it. Read the run through `gp agent` commands, not by reading these.",
        v["files"]["run"].as_str().unwrap_or(""),
        v["files"]["notes"].as_str().unwrap_or(""),
        v["files"]["agent"].as_str().unwrap_or("")
    ));
    let atts = v["attempts"].as_array().cloned().unwrap_or_default();
    if atts.is_empty() {
        line("No attempts yet.".into());
    } else {
        line("Attempts:".into());
        for a in &atts {
            line(format!(
                "- {} (folder {})",
                a["line"].as_str().unwrap_or(""),
                a["dir"].as_str().unwrap_or("")
            ));
        }
    }
    line("Next:".into());
    for n in v["next"].as_array().into_iter().flatten() {
        line(format!(
            "- {}:\n  {}",
            n["why"].as_str().unwrap_or(""),
            n["run"].as_str().unwrap_or("")
        ));
    }
    o.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_measure_path_gives_its_entity_and_field() {
        assert_eq!(
            kind_field("bodies.spiny[0].y"),
            ("spiny".into(), "y".into())
        );
        assert_eq!(
            kind_field("entities.mario[0].facing"),
            ("mario".into(), "facing".into())
        );
        assert_eq!(kind_field("camera.x"), ("camera".into(), "x".into()));
    }

    #[test]
    fn a_line_is_under_the_label_above_it() {
        let lines: Vec<String> = [
            "JumpEngine:",
            "  lda $00",
            "ImposeGravity: pha",
            "  ; a comment: with a colon",
            "  cmp #$04",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(routine_at(&lines, 2).as_deref(), Some("JumpEngine"));
        assert_eq!(routine_at(&lines, 5).as_deref(), Some("ImposeGravity"));
    }

    #[test]
    fn arguments_are_quoted_only_when_they_need_it() {
        assert_eq!(q("/a/b.nes"), "/a/b.nes");
        assert_eq!(
            q("/roms/Super Mario (PRG 0).nes"),
            "\"/roms/Super Mario (PRG 0).nes\""
        );
        assert_eq!(q("say \"hi\" $x"), "\"say \\\"hi\\\" \\$x\"");
    }
}
