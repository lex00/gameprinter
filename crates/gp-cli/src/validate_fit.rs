//! `gp validate GAME` with neither `--against` nor `--store`: the check
//! that fits what the game's folder holds, chosen and said in words
//! (issue #7; docs/validation.md, "The check that fits a game").
//!
//! | The folder | The check |
//! |---|---|
//! | Its source has recordings of the original in the checkout's `baselines/<source>` (Moss Hop) | The fidelity contract against them, as `--against baselines/<source>` runs it; for a person's copy, the changed-game check too, which decides `ok` and the exit code, with the contract's result beside it |
//! | It holds a corpus (`corpus.toml`) and is an original: in a git work tree, or a published package | A port's: its corpus scored on the model (`gp corpus score`, as `gp game score` runs it), on NES and arcade alike |
//! | Anything else: a person's copy, or a game made here | A changed or new game's: the model is well formed (`gp model check`) and every stage can be reached and finished |
//!
//! A game someone changed on purpose (a higher jump, a faster barrel) is
//! not held to recordings of the original: they would read the change as
//! a failure. Its stages are checked by the tools that already check one:
//! in a game with a level kit, `gp level verify` for a girder stage and a
//! tile stage (the kit's rules where it covers the stage, and the stage
//! played to its end with the enemies in: the direct route up the
//! ladders, the search that rides carriers and plays to a condition, or
//! the reach search and the playthrough on tiles), and in a game with no
//! kit the reach search and the playthrough (`gp interp reach`,
//! `gp_interp::reach`). A stage none of them can play to its end is
//! named NOT CHECKED, with the reason.

use crate::corpus_score::Picture;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gp_evidence::Paths;
use gp_model::{MODEL_FILE, Model};
use serde_json::{Value, json};

use crate::Outcome;
use crate::validate::ValidateArgs;

/// Which check fits a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fit {
    /// The fidelity contract against recordings of the original.
    Contract,
    /// A port's: its corpus scored on the model.
    Port,
    /// A game changed on purpose or made new.
    Changed,
}

impl Fit {
    fn word(self) -> &'static str {
        match self {
            Fit::Contract => "contract",
            Fit::Port => "port",
            Fit::Changed => "changed",
        }
    }
}

/// What the folder holds that the choice reads.
#[derive(Debug, Clone, Default)]
pub(crate) struct Holds {
    /// The model's source, when it names one.
    pub source: Option<String>,
    /// The checkout's folder of recordings of the original for the source,
    /// and how many it holds.
    pub baselines: Option<(PathBuf, usize)>,
    /// `corpus.toml` is in the folder.
    pub corpus: bool,
    /// An original: in a git work tree, or a published package as gp game
    /// fetch writes one. Else a person's copy or a folder of their own.
    pub original: bool,
    /// Where the folder is an original, the words that say so.
    pub original_why: Option<String>,
}

/// The choice and its reason, in words.
pub(crate) fn choose(name: &str, h: &Holds) -> (Fit, Option<Fit>, String) {
    if let Some((dir, n)) = &h.baselines {
        let also = (!h.original).then_some(Fit::Changed);
        let mut why = format!(
            "the checkout has {n} recording{} of the original game {} in {}, so the check is {name}'s fidelity contract against them, as gp validate --against that folder runs it",
            if *n == 1 { "" } else { "s" },
            h.source.as_deref().unwrap_or("?"),
            dir.display(),
        );
        if also.is_some() {
            why.push_str(". This folder is a person's copy (it is in no git work tree), and a change made on purpose reads against the original as a failure, so the check for a changed game runs too, and decides `ok` and the exit code: the model well formed, every stage reached and finished. The contract's result is beside it under `contract`, as how far the copy now is from the original");
        }
        return (Fit::Contract, also, why);
    }
    if h.corpus && h.original {
        return (
            Fit::Port,
            None,
            format!(
                "{name} is a port: its folder holds a corpus (corpus.toml) of clips of the original, and {}. The check is its corpus scored on the model (gp corpus score), every clip exact",
                h.original_why.as_deref().unwrap_or("it is an original")
            ),
        );
    }
    let what = if h.corpus {
        format!(
            "{name} is a person's copy of a port (it holds the port's corpus.toml, and it is in no git work tree), changed on purpose or about to be. The clips of the original would read a change as a failure, so"
        )
    } else if h.original {
        format!(
            "{name} is a game with no recordings of an original: no corpus.toml, and no baselines/{} in the checkout. So",
            h.source.as_deref().unwrap_or("SOURCE")
        )
    } else {
        format!(
            "{name} is a person's copy or a game of their own (it is in no git work tree, and the checkout has no recordings of its original), so"
        )
    };
    (
        Fit::Changed,
        None,
        format!(
            "{what} the check is the one for a game changed on purpose or made new: the model well formed (gp model check), and every stage reached and finished (gp level verify, gp interp reach and its playthrough)"
        ),
    )
}

/// The folder's facts the choice reads.
pub(crate) fn holds(dir: &Path, model: Option<&Model>) -> Holds {
    let source = model.and_then(|m| m.model.source.clone());
    let baselines = source.as_deref().and_then(original_baselines);
    let package = dir.join("lifted").is_dir() && dir.join("package.toml").is_file();
    let git = gp_evidence::paths::enclosing_git_work_tree(dir);
    let original_why = match (&git, package) {
        (Some(g), _) => Some(format!("it is in the git work tree {}", g.display())),
        (None, true) => Some("it is a published package as gp game fetch writes one".into()),
        (None, false) => None,
    };
    Holds {
        source,
        baselines,
        corpus: dir.join(crate::corpus_score::FILE).is_file(),
        original: original_why.is_some(),
        original_why,
    }
}

/// The checkout's recordings of the original for `source`
/// (`baselines/<source>/`), when there are any.
fn original_baselines(source: &str) -> Option<(PathBuf, usize)> {
    if source.is_empty() || source.contains(['/', '\\', '.']) {
        return None;
    }
    let repo = crate::repo::find_repo(|d| d.join("baselines").join(source).is_dir())?;
    let dir = repo.join("baselines").join(source);
    let n = gp_evidence::baseline::list(&dir).ok()?.len();
    (n > 0).then_some((dir, n))
}

pub(crate) fn run(args: &ValidateArgs, paths: &Paths, which: bool) -> Result<Outcome> {
    let dir = crate::level_edit::resolve(&args.model, paths);
    if !dir.join(MODEL_FILE).is_file() {
        anyhow::bail!(
            "{} isn't a game's folder: it has no {MODEL_FILE}. Name a game's folder, a copy (template_fork's name) or a game in the games checkout",
            dir.display()
        );
    }
    let dir = std::path::absolute(&dir)?;
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("the game")
        .to_owned();
    let text = fs::read_to_string(dir.join(MODEL_FILE))
        .with_context(|| dir.join(MODEL_FILE).display().to_string())?;
    let model = Model::parse(&text).ok();
    let targets = args
        .targets
        .iter()
        .map(|t| crate::validate_target::parse(t))
        .collect::<Result<Vec<_>>>()?;
    let h = holds(&dir, model.as_ref());
    let (fit, also, why) = choose(&name, &h);
    let head = json!({
        "game": name, "dir": dir, "check": fit.word(),
        "also": also.map(Fit::word), "why": why,
    });
    if which {
        let mut j = head;
        j["ok"] = json!(true);
        return Ok(Outcome::ok(j, format!("{name}: {why}.")));
    }
    match fit {
        Fit::Contract => {
            let (base, _) = h.baselines.clone().context("baselines were found")?;
            let contract = ValidateArgs {
                model: dir.clone(),
                against: vec![base.display().to_string()],
                store: false,
                runtime: args.runtime,
                module: args.module.clone(),
                assets: args.assets.clone(),
                rom: args.rom.clone(),
                mapping: args.mapping.clone(),
                no_save: args.no_save,
                power_on: args.power_on,
                which: false,
                targets: Vec::new(),
            };
            let o = crate::validate::run(&contract, paths)?;
            let mut json = o.json;
            for (k, v) in head.as_object().into_iter().flatten() {
                json[k] = v.clone();
            }
            let mut text = format!("{name}: {why}.\n{}", o.text);
            let mut exit = o.exit;
            if also.is_none() && !targets.is_empty() {
                text.push_str("\nThe targets weren't held: this is the original game, held to its recordings. A copy of it (template_fork) is checked as a changed game, and holds them");
                json["difficulty"] =
                    json!({"held": false, "why": "the original game is held to its recordings"});
            }
            if also == Some(Fit::Changed) {
                // A person's copy: the changed game's check decides, and the
                // contract says how far the change moved it.
                let c = changed(&dir, &name, model.as_ref(), args, &targets, paths)?;
                let says = against_original(o.exit);
                json["contract"] = json!({
                    "ok": json["ok"].clone(), "exit": o.exit,
                    "passed": json["passed"].clone(), "failed": json["failed"].clone(),
                    "says": says,
                });
                json["ok"] = c.json["ok"].clone();
                json["decided_by"] = json!("changed");
                text.push_str("\n\nThe check for a changed game:\n");
                text.push_str(&c.text);
                text.push_str(&format!(
                    "\nAs a changed game: {}; this decides the result. Against the original: {says}.",
                    if c.exit == 0 { "PASS" } else { "FAIL" },
                ));
                exit = c.exit;
                json["changed"] = c.json;
            }
            Ok(Outcome { json, text, exit })
        }
        Fit::Port => {
            let o = port(&dir, &name, args, paths)?;
            let mut o = merge(head, &why, &name, o, "port");
            if !targets.is_empty() {
                not_held(
                    &mut o,
                    "a port is held to its clips of the original, every one exact, so its difficulty is the original's",
                );
            }
            Ok(o)
        }
        Fit::Changed => {
            let mut o = changed(&dir, &name, model.as_ref(), args, &targets, paths)?;
            // A copy of a port still has the port's clips: what they say is
            // how far the change moved it from the original.
            if h.corpus && o.json["complete"] == false && o.json["well_formed"]["ok"] == true {
                o.text.push_str(&format!(
                    "\nIts clips of the original are still here: gp corpus score {} --run {name} says how far it now is from the original, clip by clip, where the change should show",
                    dir.display()
                ));
            }
            Ok(merge(head, &why, &name, o, "changed"))
        }
    }
}

/// What the contract's exit says of a person's copy, beside the changed
/// game's check that decides its result.
fn against_original(exit: u8) -> &'static str {
    match exit {
        0 => "the same as the original",
        1 => "differs from the original, as you changed it",
        _ => "not compared: no recording of the original could be run",
    }
}

/// Targets given where the check that fits doesn't hold them.
fn not_held(o: &mut Outcome, why: &str) {
    o.text.push_str(&format!(
        "\nThe targets weren't held: {why}. A copy of it (template_fork) is checked as a changed game, and holds them"
    ));
    o.json["difficulty"] = json!({"held": false, "why": why});
}

fn merge(head: Value, why: &str, name: &str, o: Outcome, key: &str) -> Outcome {
    let mut json = head;
    json["ok"] = o.json["ok"].clone();
    json[key] = o.json;
    Outcome {
        json,
        text: format!("{name}: {why}.\n{}", o.text),
        exit: o.exit,
    }
}

// --- A port ---------------------------------------------------------------

/// A port's check: its corpus scored on the model, as `gp game score`
/// scores it (the game's own module, built from this gp's kit when it has
/// hooks), every clip exact.
fn port(dir: &Path, name: &str, args: &ValidateArgs, paths: &Paths) -> Result<Outcome> {
    let out = paths
        .home
        .join("work")
        .join("validate")
        .join(name)
        .join("score.json");
    let o = crate::corpus_score::game_score(paths, dir, args.module.as_deref(), &out)?;
    let arcade = crate::publish_rule::is_arcade_game(dir);
    Ok(port_verdict(name, arcade, &o.json, &o.text, &out))
}

/// The port's verdict from its corpus score (`score` is `gp corpus score`'s
/// JSON, `text` its summary): state and picture apart. Every clip exact on
/// state passes a port, NES or arcade, as the publish rule asks
/// (crate::publish_rule); the picture is said, and doesn't decide.
fn port_verdict(name: &str, arcade: bool, score: &Value, text: &str, out: &Path) -> Outcome {
    let r = &score["record"]["predicate"]["results"];
    let n = r["clips"].as_u64().unwrap_or(0);
    let exact = r["exact"].as_u64().unwrap_or(0);
    let errors = r["errors"].as_u64().unwrap_or(0);
    let permille = r["fidelity_permille"].as_u64().unwrap_or(0);
    let rows = score["record"]["predicate"]["clips"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let not_exact: Vec<Value> = rows
        .iter()
        .filter(|c| c["exact"] != true)
        .map(|c| {
            json!({
                "id": c["id"], "permille": c["permille"], "error": c.get("error"),
                "first": c.get("first"),
            })
        })
        .collect();
    // The picture, from each scored clip's screen counts.
    let pictures: Vec<(String, Picture)> = rows
        .iter()
        .filter(|c| c.get("error").is_none_or(Value::is_null))
        .map(|c| (c["id"].as_str().unwrap_or("?").to_owned(), Picture::of(c)))
        .collect();
    let (same, frames) = pictures
        .iter()
        .fold((0, 0), |(s, f), (_, pc)| (s + pc.same, f + pc.frames));
    let picture_off: Vec<&str> = pictures
        .iter()
        .filter(|(_, pc)| !pc.exact())
        .map(|(id, _)| id.as_str())
        .collect();
    let picture_ok = n > 0 && pictures.len() as u64 == n && picture_off.is_empty();
    let problems = score["problems"].as_array().cloned().unwrap_or_default();
    let state_ok = n > 0 && exact == n && errors == 0 && problems.is_empty();
    let ok = state_ok;
    let mut lines = vec![text.to_owned()];
    for c in not_exact.iter().take(20) {
        let id = c["id"].as_str().unwrap_or("");
        let said = match c["error"].as_str() {
            Some(e) => format!("couldn't be scored: {e}"),
            None => {
                let p = c["permille"].as_u64().unwrap_or(0);
                let first = c["first"]
                    .as_object()
                    .map(|f| {
                        format!(
                            "; first differs at frame {} on {}",
                            f.get("frame").and_then(Value::as_u64).unwrap_or(0),
                            f.get("path").and_then(Value::as_str).unwrap_or("?")
                        )
                    })
                    .unwrap_or_default();
                format!("{}.{}%{first}", p / 10, p % 10)
            }
        };
        lines.push(format!("  not exact on state: {id}: {said}"));
    }
    if not_exact.len() > 20 {
        lines.push(format!("  and {} more", not_exact.len() - 20));
    }
    for (id, pc) in pictures.iter().filter(|(_, pc)| !pc.exact()).take(20) {
        lines.push(format!(
            "  differs on picture: {id}: {} of {} screen frames the same",
            pc.same, pc.frames
        ));
    }
    if picture_off.len() > 20 {
        lines.push(format!("  and {} more", picture_off.len() - 20));
    }
    let picture = format!("{same} of {frames} screen frames the same");
    let state = if n == 0 {
        format!(
            "FAIL: {name}'s corpus has no clips to score yet (gp corpus import makes them from the game's sessions)"
        )
    } else if state_ok {
        format!("PASS on state: every one of {name}'s {n} clips is exact on the model")
    } else {
        format!(
            "FAIL on state: {} of {n} clips aren't exact; gp clip score {name} CLIP shows where one splits",
            n - exact
        )
    };
    lines.push(state);
    if n > 0 {
        lines.push(if picture_ok {
            format!("PASS on picture: {picture}, every one of the {n} clips")
        } else {
            format!(
                "On picture: {picture}, {} of {n} clips differ (said, not decided: a port passes on state)",
                n.saturating_sub(pictures.len() as u64 - picture_off.len() as u64)
            )
        });
    }
    Outcome {
        json: json!({
            "ok": ok, "clips": n, "exact": exact, "errors": errors,
            "fidelity_permille": permille, "not_exact": not_exact,
            "state_ok": state_ok,
            "picture": {"ok": picture_ok, "frames_same": same, "frames": frames,
                "differ": picture_off, "decides": false, "arcade": arcade},
            "problems": problems, "record_file": out,
        }),
        text: lines.join("\n"),
        exit: u8::from(!ok),
    }
}

// --- A changed or new game ------------------------------------------------

/// The lifted assets a check of the model's slots reads: the folder's own,
/// else `--assets`, else the source's in this home, when there.
fn assets_for(
    dir: &Path,
    model: Option<&Model>,
    args: &ValidateArgs,
    paths: &Paths,
) -> Option<PathBuf> {
    let own = dir.join("assets");
    if own.join(gp_assets::MAP_FILE).is_file() {
        return Some(own);
    }
    if let Some(a) = &args.assets {
        return Some(a.clone());
    }
    let s = model?.model.source.as_ref()?;
    let shared = paths.home.join("work").join("assets").join(s);
    shared.join(gp_assets::MAP_FILE).is_file().then_some(shared)
}

/// The folder a model's evidence refs are read against: the game's own
/// for a package, else the git work tree it is in, else (a person's copy
/// of a template, which keeps the template's refs) the checkout, as the
/// MCP server's model check reads them.
fn ref_root(dir: &Path) -> Option<PathBuf> {
    if dir.join("package.toml").is_file()
        || gp_evidence::paths::enclosing_git_work_tree(dir).is_some()
    {
        return None;
    }
    crate::repo::find_repo(|d| d.join("templates").is_dir() && d.join("baselines").is_dir())
}

/// One stage's result.
struct Stage {
    level: String,
    /// Some(true) checked and passed, Some(false) failed, None not checked.
    ok: Option<bool>,
    how: &'static str,
    said: String,
    detail: Value,
    /// Its numbers by name, for --target.
    measured: crate::validate_target::Measured,
}

/// A changed or new game's check: the model well formed, then each stage
/// reached and finished by the tools that check a stage.
fn changed(
    dir: &Path,
    name: &str,
    model: Option<&Model>,
    args: &ValidateArgs,
    targets: &[crate::validate_target::Target],
    paths: &Paths,
) -> Result<Outcome> {
    let assets = assets_for(dir, model, args, paths);
    let root = ref_root(dir);
    let mc = crate::model::check_in(dir, assets.as_deref(), root.as_deref(), &paths.home)?;
    let well_formed = mc.exit == 0;
    let mut lines = vec![if well_formed {
        format!(
            "well formed: yes ({})",
            mc.text.lines().last().unwrap_or("gp model check: valid")
        )
    } else {
        format!(
            "well formed: NO, gp model check found {} problems:\n  {}",
            mc.json["problems"].as_array().map_or(0, Vec::len),
            mc.text.lines().collect::<Vec<_>>().join("\n  ")
        )
    }];
    let mut stages = Vec::new();
    if let (true, Some(m)) = (well_formed, model) {
        stages = check_stages(dir, m, args, targets, paths);
    } else {
        lines.push("stages: not checked, since the model isn't well formed".into());
    }
    for s in &stages {
        let state = match s.ok {
            Some(true) => "ok    ",
            Some(false) => "FAIL  ",
            None => "NOT CHECKED ",
        };
        lines.push(format!("{state}stage {} ({}): {}", s.level, s.how, s.said));
    }
    let failed: Vec<&str> = stages
        .iter()
        .filter(|s| s.ok == Some(false))
        .map(|s| s.level.as_str())
        .collect();
    let unchecked: Vec<&str> = stages
        .iter()
        .filter(|s| s.ok.is_none())
        .map(|s| s.level.as_str())
        .collect();
    let mut ok = well_formed && failed.is_empty();
    let mut complete = well_formed && unchecked.is_empty();
    let mut difficulty = Value::Null;
    if !targets.is_empty() {
        if well_formed {
            let measured: Vec<_> = stages.iter().map(|s| s.measured.clone()).collect();
            let (j, l, met, all) = crate::validate_target::hold(targets, &measured);
            lines.extend(l);
            ok &= met;
            complete &= all;
            difficulty = j;
        } else {
            difficulty = json!({"held": false, "why": "the model isn't well formed"});
        }
    }
    let off: usize = difficulty["targets"]
        .as_array()
        .map_or(0, |r| r.iter().filter(|r| r["ok"] == false).count());
    let unmeasured: usize = difficulty["targets"]
        .as_array()
        .map_or(0, |r| r.iter().filter(|r| r["measured"] == false).count());
    lines.push(if !well_formed {
        "FAIL: the model isn't well formed; fix what gp model check names first".into()
    } else if !failed.is_empty() {
        format!(
            "FAIL: {} of {} stages can't be shown to be reached and finished: {}",
            failed.len(),
            stages.len(),
            failed.join(", ")
        )
    } else if off > 0 {
        format!(
            "FAIL: every stage can be reached and finished, but {off} of the measures you set a target for {} off it",
            if off == 1 { "is" } else { "are" }
        )
    } else if unmeasured > 0 {
        format!(
            "PASS on what could be checked, NOT a full check: nothing failed, but {unmeasured} of your targets couldn't be measured{}",
            if unchecked.is_empty() {
                String::new()
            } else {
                format!(", and no tool here could search {}", unchecked.join(", "))
            }
        )
    } else if unchecked.is_empty() {
        format!(
            "PASS: {name}'s model is well formed and {} can be reached and finished{}",
            if stages.len() == 1 {
                "its one stage".to_owned()
            } else {
                format!("all {} stages", stages.len())
            },
            if targets.is_empty() {
                ""
            } else {
                ", and every measure you set a target for is on it"
            }
        )
    } else {
        format!(
            "PASS on what could be checked, NOT a full check: the model is well formed and nothing failed, but no tool here could search {} of {} stages ({}), so nothing says they can be finished",
            unchecked.len(),
            stages.len(),
            unchecked.join(", ")
        )
    });
    let stage_json: Vec<Value> = stages
        .iter()
        .map(|s| {
            json!({"level": s.level, "ok": s.ok, "how": s.how, "said": s.said, "detail": s.detail})
        })
        .collect();
    Ok(Outcome {
        json: json!({
            "ok": ok, "complete": complete,
            "well_formed": {"ok": well_formed, "problems": mc.json["problems"]},
            "stages": stage_json, "failed": failed, "unchecked": unchecked,
            "difficulty": difficulty,
        }),
        text: lines.join("\n"),
        exit: u8::from(!ok),
    })
}

/// Every stage of the model, each by the tool that can check it.
fn check_stages(
    dir: &Path,
    model: &Model,
    args: &ValidateArgs,
    targets: &[crate::validate_target::Target],
    paths: &Paths,
) -> Vec<Stage> {
    // gp level list's view: each level's kind, and whether the kit covers it.
    let listed = crate::level_edit::list_cmd(dir, paths).ok().map(|o| o.json);
    let kit = listed.as_ref().is_some_and(|l| l["kit"] == true);
    let covered: Vec<String> = listed
        .as_ref()
        .and_then(|l| l["levels"].as_array())
        .into_iter()
        .flatten()
        .filter(|l| l["kit_covers"] == true)
        .filter_map(|l| l["level"].as_str().map(str::to_owned))
        .collect();
    let kind_of = |level: &str| -> String {
        listed
            .as_ref()
            .and_then(|l| l["levels"].as_array())
            .into_iter()
            .flatten()
            .find(|l| l["level"] == level)
            .and_then(|l| l["kind"].as_str())
            .unwrap_or("")
            .to_owned()
    };
    // The model as the tools run it, opened once, for the reach search.
    let source =
        crate::hosted::Source::open_dir(dir, args.assets.as_deref(), paths, args.module.as_deref())
            .map(|(s, _)| s)
            .map_err(|e| format!("{e:#}"));
    let mut out = Vec::new();
    for level in model.levels.keys() {
        if kit && ["girders and ladders", "tiles"].contains(&kind_of(level).as_str()) {
            out.push(verify_stage(
                dir,
                level,
                covered.contains(level),
                &covered,
                args,
                paths,
            ));
            continue;
        }
        out.push(match &source {
            Ok(s) => reach_stage(s, level),
            Err(e) => Stage {
                level: level.clone(),
                ok: None,
                how: "reach and playthrough",
                said: format!("the model couldn't be opened to search it: {e}"),
                detail: Value::Null,
                measured: Default::default(),
            },
        });
    }
    // The numbers a target names that cost a search of their own. They
    // come from the reach search's moves (a platformer's walk and jump on
    // tiles), so a girder stage, whose climbs and jumps the stage tools
    // play instead, doesn't get them.
    for s in &mut out {
        s.measured.level = s.level.clone();
        if kit && kind_of(&s.level) == "girders and ladders" {
            for t in targets {
                if crate::validate_target::ON_DEMAND.contains(&t.name.as_str()) {
                    s.measured.not.insert(
                        t.name.clone(),
                        "it is measured by the reach search, which moves a platformer's player on tiles; this is a stage of girders and ladders, which the reach search doesn't play".into(),
                    );
                }
            }
            continue;
        }
        match &source {
            Ok(src) => crate::validate_target::on_demand(src, &s.level, targets, &mut s.measured),
            Err(e) => {
                for t in targets {
                    if crate::validate_target::ON_DEMAND.contains(&t.name.as_str()) {
                        s.measured
                            .not
                            .insert(t.name.clone(), format!("the model couldn't be opened: {e}"));
                    }
                }
            }
        }
    }
    out
}

/// A girder or tile stage of a game with a level kit, by `gp level
/// verify`: the kit's rules where the kit covers it, and the stage played
/// to its end with the enemies in, as the game reaches it
/// (gp_interp::stage::enter). A stage verify can't play to its end is
/// NOT CHECKED, with its words. How far the layout is from the
/// original's is said, not held: a stage left as the original's can
/// still be played.
fn verify_stage(
    dir: &Path,
    level: &str,
    covers: bool,
    covered: &[String],
    args: &ValidateArgs,
    paths: &Paths,
) -> Stage {
    let how = "level verify";
    let r = crate::level_edit::verify_cmd(dir, Some(level), 6, 3000, args.module.as_deref(), paths);
    let o = match r {
        Ok(o) => o,
        Err(e) => {
            return Stage {
                level: level.into(),
                ok: None,
                how,
                said: format!("gp level verify couldn't check it: {e:#}"),
                detail: Value::Null,
                measured: Default::default(),
            };
        }
    };
    verified(dir, level, covers, covered, &o.json)
}

/// [`verify_stage`]'s stage from what gp level verify returned (`j`, its
/// JSON).
fn verified(dir: &Path, level: &str, covers: bool, covered: &[String], j: &Value) -> Stage {
    let how = "level verify";
    let finished = j["finished"]["finished"] == true;
    let checked = j["finished"]["checked"] != false;
    let breaks = j["breaks"].as_array().map_or(0, Vec::len);
    let mut said = if !checked {
        format!(
            "whether it can be finished is NOT CHECKED: {}",
            j["finished"]["why"]
                .as_str()
                .unwrap_or("gp level verify couldn't play it")
        )
    } else if let Some(said) = j["finished"]["said"].as_str() {
        said.to_owned()
    } else if finished {
        format!(
            "the direct route played with the enemies in reaches the goal at frame {}",
            j["finished"]["frame"]
        )
    } else {
        format!(
            "the direct route played with the enemies in never reaches the goal{}",
            j["finished"]["why"]
                .as_str()
                .map(|w| format!(" ({w})"))
                .unwrap_or_default()
        )
    };
    let ok = if covers {
        if breaks == 0 {
            said.push_str("; the kit's rules: kept");
        } else {
            said.push_str(&format!(
                "; it breaks {breaks} of the kit's rules (gp level verify {} --level {level} names them)",
                dir.display()
            ));
        }
        if breaks > 0 {
            Some(false)
        } else if !checked {
            None
        } else {
            Some(finished)
        }
    } else {
        said.push_str(&format!(
            "; the level kit's rules are written for {}, not this stage, so they aren't held here",
            if covered.is_empty() {
                "other stages".to_owned()
            } else {
                covered.join(", ")
            }
        ));
        checked.then_some(finished)
    };
    if j["original"]["apart"] == false {
        said.push_str(
            "; its layout is the original's or close to it, which gp level verify refuses for a new stage and which doesn't stop it being played",
        );
    }
    Stage {
        level: level.into(),
        ok,
        how,
        said,
        detail: json!({
            "finished": j["finished"], "breaks": j["breaks"], "kit_covers": covers,
            "original": j["original"], "picture": j["picture"],
        }),
        // The kit's tolerances only where its rules are written for this
        // stage.
        measured: crate::validate_target::Measured {
            level: level.into(),
            numbers: crate::validate_target::numbers_of(&j["measured"]),
            kit: if covers {
                j["measures"].as_array().cloned().unwrap_or_default()
            } else {
                Vec::new()
            },
            not: Default::default(),
        },
    }
}

/// Any other stage, by the reach search and the playthrough
/// (`gp interp reach`): the goal reachable by the player alone, then a run
/// to it with every enemy in.
fn reach_stage(src: &crate::hosted::Source, level: &str) -> Stage {
    let how = "reach and playthrough";
    let stage = |ok, said: String, detail: Value| Stage {
        level: level.into(),
        ok,
        how,
        said,
        measured: crate::validate_target::Measured {
            level: level.into(),
            numbers: detail["frames"]
                .as_f64()
                .map(|f| ("run_frames".to_owned(), f))
                .into_iter()
                .collect(),
            ..Default::default()
        },
        detail,
    };
    let spec = &src.spec;
    let Some(i) = spec.levels.iter().position(|l| l.name == level) else {
        return stage(
            None,
            "the interpreter has no such level".into(),
            Value::Null,
        );
    };
    let r = match gp_interp::reach::reach(spec, i, src) {
        Ok(r) => r,
        Err(e) => {
            return stage(
                None,
                format!(
                    "the reach search can't search it ({e}): it moves a platformer's player by its walk and jump, and no other tool here checks this kind of stage yet"
                ),
                Value::Null,
            );
        }
    };
    if r.goal_path.is_none() {
        return stage(
            Some(false),
            format!(
                "the goal is NOT reachable by the player alone: {} places to stand, {} moves searched",
                r.stands.len(),
                r.moves
            ),
            json!({"reachable": false, "stands": r.stands.len(), "moves": r.moves}),
        );
    }
    match gp_interp::reach::playthrough(spec, i, src) {
        Ok(Some(p)) => stage(
            Some(true),
            format!(
                "the goal is reachable by the player alone ({} places to stand), and a run with every enemy in reaches it in {} frames",
                r.stands.len(),
                p.buttons.len()
            ),
            json!({"reachable": true, "stands": r.stands.len(), "finished": true,
                "frames": p.buttons.len(), "searched": p.searched}),
        ),
        Ok(None) => stage(
            Some(false),
            format!(
                "the goal is reachable by the player alone, but no run with the enemies in was found in {} places",
                gp_interp::reach::PLAYTHROUGH_LIMIT
            ),
            json!({"reachable": true, "finished": false}),
        ),
        Err(e) => stage(
            None,
            format!("the goal is reachable by the player alone; the playthrough couldn't run: {e}"),
            json!({"reachable": true}),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up score of two clips, both exact on state, the second with
    /// `second` of 10 screen frames the same.
    fn made_up_score(second: u64) -> Value {
        let screen = |same: u64| json!({"sprites": {"same": same, "total": 10}, "background": {"same": 10, "total": 10}});
        json!({"problems": [], "record": {"predicate": {
        "results": {"clips": 2, "exact": 2, "errors": 0, "fidelity_permille": 1000},
        "clips": [
            {"id": "a.1", "exact": true, "permille": 1000, "screen": screen(10)},
            {"id": "a.2", "exact": true, "permille": 1000, "screen": screen(second)},
        ]}}})
    }

    #[test]
    fn a_port_says_state_and_picture_apart_and_state_decides() {
        let out = Path::new("made-up/score.json");
        // Exact on state, a frame different on picture: an arcade game
        // passes, the picture said.
        let o = port_verdict("cab", true, &made_up_score(9), "summary", out);
        assert_eq!(
            (o.json["ok"].as_bool(), o.exit),
            (Some(true), 0),
            "{}",
            o.text
        );
        assert!(
            o.text
                .contains("PASS on state: every one of cab's 2 clips is exact")
                && o.text.contains(
                    "On picture: 19 of 20 screen frames the same, 1 of 2 clips differ (said, not decided"
                )
                && o.text
                    .contains("differs on picture: a.2: 9 of 10 screen frames the same"),
            "{}",
            o.text
        );
        assert_eq!(o.json["picture"]["differ"], json!(["a.2"]));
        // An NES port: the picture is said, and state decides.
        let o = port_verdict("nes", false, &made_up_score(9), "summary", out);
        assert_eq!(
            (o.json["ok"].as_bool(), o.exit),
            (Some(true), 0),
            "{}",
            o.text
        );
        assert!(
            o.text.contains("On picture: 19 of 20 screen frames the same, 1 of 2 clips differ (said, not decided"),
            "{}",
            o.text
        );
        // Every screen frame the same: an arcade game passes on both.
        let o = port_verdict("cab", true, &made_up_score(10), "summary", out);
        assert_eq!(
            (o.json["ok"].as_bool(), o.exit),
            (Some(true), 0),
            "{}",
            o.text
        );
        assert!(
            o.text
                .contains("PASS on picture: 20 of 20 screen frames the same"),
            "{}",
            o.text
        );
        // No screen counts at all: an arcade game still passes on state,
        // the picture said.
        let mut none = made_up_score(10);
        for c in none["record"]["predicate"]["clips"].as_array_mut().unwrap() {
            c.as_object_mut().unwrap().remove("screen");
        }
        let o = port_verdict("cab", true, &none, "summary", out);
        assert_eq!(o.json["ok"], true, "{}", o.text);
        assert!(
            o.text
                .contains("On picture: 0 of 0 screen frames the same, 2 of 2 clips differ"),
            "{}",
            o.text
        );
    }

    #[test]
    fn a_stage_verify_couldnt_play_is_not_checked_and_one_it_played_counts() {
        let dir = Path::new("made-up");
        let covered = vec!["first".to_owned()];
        // Played to its goal past the stage before: it counts.
        let played = json!({"finished": {"finished": true, "checked": true, "frame": 900,
            "entered_after": ["first"]}, "breaks": [], "original": {"apart": true}});
        let s = verified(dir, "second", false, &covered, &played);
        assert_eq!(s.ok, Some(true), "{}", s.said);
        assert!(
            s.said.contains("reaches the goal at frame 900"),
            "{}",
            s.said
        );
        // Never reached: it fails.
        let failed = json!({"finished": {"finished": false, "checked": true, "why": "no waits"},
            "breaks": [], "original": {"apart": true}});
        assert_eq!(
            verified(dir, "second", false, &covered, &failed).ok,
            Some(false)
        );
        // Not played (no goal): not checked, with verify's words, whether
        // the kit covers it or not, unless a rule of the kit breaks.
        let unplayed = json!({"finished": {"finished": null, "checked": false,
            "why": "level second has no goal to climb to"}, "breaks": [], "original": {"apart": true}});
        for covers in [false, true] {
            let s = verified(dir, "second", covers, &covered, &unplayed);
            assert_eq!(s.ok, None, "{}", s.said);
            assert!(
                s.said.contains("NOT CHECKED: level second has no goal"),
                "{}",
                s.said
            );
        }
        let mut broken = unplayed.clone();
        broken["breaks"] = json!([{"rule": "a rule", "said": "broken"}]);
        assert_eq!(
            verified(dir, "second", true, &covered, &broken).ok,
            Some(false)
        );
    }

    fn holds(baselines: bool, corpus: bool, original: bool) -> Holds {
        Holds {
            source: Some("made-up".into()),
            baselines: baselines.then(|| (PathBuf::from("baselines/made-up"), 3)),
            corpus,
            original,
            original_why: original.then(|| "it is in the git work tree /x".into()),
        }
    }

    #[test]
    fn the_check_is_chosen_from_what_the_folder_holds() {
        // Recordings of the original: the contract, as --against runs it.
        let (f, also, why) = choose("g", &holds(true, false, true));
        assert_eq!((f, also), (Fit::Contract, None));
        assert!(
            why.contains("3 recordings") && why.contains("--against"),
            "{why}"
        );
        // A person's copy of such a game: the changed check beside it.
        let (f, also, why) = choose("g", &holds(true, false, false));
        assert_eq!((f, also), (Fit::Contract, Some(Fit::Changed)));
        assert!(
            why.contains("person's copy") && why.contains("decides `ok` and the exit code"),
            "{why}"
        );
        assert_eq!(
            against_original(1),
            "differs from the original, as you changed it"
        );
        assert_eq!(against_original(0), "the same as the original");
        // An original with a corpus: a port.
        let (f, also, why) = choose("g", &holds(false, true, true));
        assert_eq!((f, also), (Fit::Port, None));
        assert!(why.contains("gp corpus score"), "{why}");
        // A copy of a port: changed on purpose.
        let (f, _, why) = choose("g", &holds(false, true, false));
        assert_eq!(f, Fit::Changed);
        assert!(why.contains("copy of a port"), "{why}");
        // A game made here, in the checkout, with nothing recorded.
        let (f, _, why) = choose("g", &holds(false, false, true));
        assert_eq!(f, Fit::Changed);
        assert!(why.contains("no recordings"), "{why}");
    }

    #[test]
    fn a_folder_outside_git_is_a_copy_and_one_inside_is_an_original() {
        let t = tempfile::tempdir().unwrap();
        let copy = t.path().join("mine");
        fs::create_dir_all(&copy).unwrap();
        fs::write(copy.join(crate::corpus_score::FILE), "").unwrap();
        let h = super::holds(&copy, None);
        assert!(h.corpus && !h.original, "{h:?}");
        // A published package as gp game fetch writes it is an original.
        fs::create_dir_all(copy.join("lifted")).unwrap();
        fs::write(copy.join("package.toml"), "").unwrap();
        let h = super::holds(&copy, None);
        assert!(h.original, "{h:?}");
        // This crate's folder is in the checkout's git work tree.
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(super::holds(here, None).original);
    }
}
