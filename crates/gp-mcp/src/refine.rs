//! `model_refine` and `model_questions`: the refinement loop and its
//! questions for a working copy (docs/mechanics.md). The baselines of the
//! copy's source are recorded into the server's store with frame traces,
//! so `gp model refine` can track and measure them, then refine runs as a
//! `gp --json` child process on the copy, as `validate` does.

use std::ffi::OsStr;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Config, Reply, run_gp};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefineArgs {
    pub copy: String,
    /// Baselines of the model's source to measure and validate against,
    /// by name. Default: all of baselines/<source>/.
    #[serde(default)]
    pub baselines: Vec<String>,
    /// The most rounds to run (default 4).
    #[serde(default)]
    pub rounds: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuestionsArgs {
    pub copy: String,
    /// Answers as `ID=N`, choosing candidate N (from 1) of question ID.
    #[serde(default)]
    pub answers: Vec<String>,
}

fn baselines(config: &Config, source: &str, names: &[String]) -> Result<Vec<PathBuf>, String> {
    let base = config.repo.join("baselines").join(source);
    if names.is_empty() {
        let mut out: Vec<PathBuf> = std::fs::read_dir(&base)
            .map_err(|e| format!("reading {}: {e}", base.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        out.sort();
        return Ok(out);
    }
    names
        .iter()
        .map(|b| {
            if crate::is_source_name(b) {
                Ok(base.join(format!("{b}.toml")))
            } else {
                Err(format!("`{b}` isn't a baseline name"))
            }
        })
        .collect()
}

pub fn refine(config: &Config, a: RefineArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let text =
        std::fs::read_to_string(dir.join(gp_model::MODEL_FILE)).map_err(|e| e.to_string())?;
    let model = gp_model::Model::parse(&text).map_err(|e| e.to_string())?;
    let source = model
        .model
        .source
        .clone()
        .ok_or("the model names no source, so there's nothing to measure")?;
    let rom = config.rom.as_ref().ok_or(
        "model_refine records the baselines, so the server needs the source ROM (gp mcp --rom)",
    )?;
    let logs = config.work.join(".logs");
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    let files = baselines(config, &source, &a.baselines)?;
    let mut ids = Vec::new();
    for b in &files {
        let name = b.file_stem().and_then(OsStr::to_str).unwrap_or("baseline");
        let log = logs.join(format!("{source}-{name}.log"));
        run_gp(
            config,
            &[
                "input-log".as_ref(),
                "generate".as_ref(),
                "--baseline".as_ref(),
                b.as_os_str(),
                "--out".as_ref(),
                log.as_os_str(),
            ],
        )?;
        let (_, v) = run_gp(
            config,
            &[
                "record".as_ref(),
                "--rom".as_ref(),
                rom.as_os_str(),
                "--input".as_ref(),
                log.as_os_str(),
                "--name".as_ref(),
                OsStr::new(name),
            ],
        )?;
        ids.push(v["id"].as_str().ok_or("gp record gave no ID")?.to_owned());
    }
    let rounds = a.rounds.unwrap_or(4).to_string();
    let mut args: Vec<&OsStr> = vec![
        "model".as_ref(),
        "refine".as_ref(),
        dir.as_os_str(),
        "--rounds".as_ref(),
        rounds.as_ref(),
        "--recording".as_ref(),
    ];
    args.extend(ids.iter().map(OsStr::new));
    args.push("--against".as_ref());
    args.extend(files.iter().map(|f| f.as_os_str()));
    args.push("--rom".as_ref());
    args.push(rom.as_os_str());
    let assets = config.assets_of(&dir, Some(&source));
    if let Some(a) = &assets {
        args.push("--assets".as_ref());
        args.push(a.as_os_str());
    }
    let (_, v) = run_gp(config, &args)?;
    Ok(Reply::ok(v))
}

pub fn questions(config: &Config, a: QuestionsArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let mut args: Vec<&OsStr> = vec!["model".as_ref(), "questions".as_ref(), dir.as_os_str()];
    for ans in &a.answers {
        args.push("--answer".as_ref());
        args.push(OsStr::new(ans));
    }
    let (code, v): (i32, Value) = run_gp(config, &args)?;
    Ok(if code == 0 {
        Reply::ok(v)
    } else {
        Reply::refused(json!({ "error": v }))
    })
}
