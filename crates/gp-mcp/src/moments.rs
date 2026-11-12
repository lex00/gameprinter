//! `moments` and `refine_moment` (docs/point-and-tell.md, "Moments in
//! words"): the game's own moments in a capture run's recording (jumps,
//! landings, deaths, lives lost, counter changes, level changes, with what
//! the model does at each), found by what the person said ("the very first
//! jump", "this jump" with a box on a paused one), and the refinement loop
//! scoped to one, asked for so the person's workspace starts it and shows
//! it beside the two games.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use gp_interp::moments::{KINDS, Moment, find};

use crate::{Config, Reply};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MomentsArgs {
    /// The capture run the person is working in.
    pub run: String,
    /// The recording they're watching, by the name the request gives it
    /// (default: the run's first).
    #[serde(default)]
    pub recording: Option<String>,
    /// What they said, to find the moment it names: "the very first jump",
    /// "the second coin", "the last death". "this jump" with `frame`
    /// names the jump at or nearest that frame.
    #[serde(default)]
    pub said: Option<String>,
    /// The original's frame they paused on, for "this" moment.
    #[serde(default)]
    pub frame: Option<u64>,
    /// Only this kind: jump, landing, death, life_lost, counter or level.
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefineMomentArgs {
    pub run: String,
    #[serde(default)]
    pub recording: Option<String>,
    /// The moment's kind: jump, landing, death, life_lost, counter or level.
    pub kind: String,
    /// Its number among the recording's moments of that kind, from 1.
    pub n: usize,
}

fn run_dir(config: &Config, run: &str) -> Result<PathBuf, String> {
    let dir = gp_capture::run_dir(&config.home, run).map_err(|e| e.to_string())?;
    if !dir.join("run.json").is_file() {
        return Err(format!("there's no capture run `{run}`"));
    }
    Ok(dir)
}

fn run_json(dir: &Path) -> Result<Value, String> {
    let b = std::fs::read(dir.join("run.json")).map_err(|e| format!("run.json: {e}"))?;
    serde_json::from_slice(&b).map_err(|e| format!("run.json: {e}"))
}

fn recording_of(dir: &Path, given: Option<&str>) -> Result<String, String> {
    if let Some(r) = given {
        if r.is_empty()
            || !r
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("`{r}` isn't a recording name"));
        }
        return Ok(r.to_owned());
    }
    let rj = run_json(dir)?;
    rj["recordings"][0]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            dir.join("input.log")
                .is_file()
                .then(|| "session".to_owned())
        })
        .ok_or_else(|| "name the recording the person is watching".to_owned())
}

/// The run's moments in the recording: the copy the workspace keeps in
/// `moments/`, else found now with `gp interp moments` and kept there.
fn load(config: &Config, dir: &Path, recording: &str) -> Result<Value, String> {
    let file = dir.join("moments").join(format!("{recording}.json"));
    if let Some(v) = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        return Ok(v);
    }
    let rj = run_json(dir)?;
    let model = if dir.join("model").join(gp_model::MODEL_FILE).is_file() {
        dir.join("model")
    } else {
        PathBuf::from(rj["model"].as_str().unwrap_or_default())
    };
    let rom = rj["rom"]
        .as_str()
        .ok_or("the run was started without a ROM, so it has no moments")?;
    let mut args: Vec<std::ffi::OsString> = vec![
        "interp".into(),
        "moments".into(),
        model.into(),
        "--rom".into(),
        rom.into(),
    ];
    let log = if recording == "session" {
        dir.join("input.log")
    } else {
        dir.join("inputs").join(format!("{recording}.log"))
    };
    if log.is_file() {
        args.extend(["--input".into(), log.into()]);
    } else {
        args.extend(["--recording".into(), recording.into()]);
    }
    if dir.join("assets").is_dir() {
        args.extend(["--assets".into(), dir.join("assets").into()]);
    }
    let refs: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
    let (code, mut j) = crate::run_gp(config, &refs)?;
    if code != 0 {
        return Err(format!("gp interp moments exited {code}: {j}"));
    }
    j["recording"] = json!(recording);
    if let Some(p) = file.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let _ = std::fs::write(&file, serde_json::to_vec(&j).unwrap_or_default());
    Ok(j)
}

/// A moment back from its JSON, for [`find`].
fn moment_of(v: &Value) -> Option<Moment> {
    let kind = KINDS.iter().find(|k| v["kind"] == **k)?;
    Some(Moment {
        kind,
        n: usize::try_from(v["n"].as_u64()?).ok()?,
        part: v["part"].as_str().unwrap_or_default().to_owned(),
        from: v["from"].as_u64()?,
        to: v["to"].as_u64()?,
        peak: v["peak"].as_u64(),
        height: None,
        before: v["before"].as_str().map(str::to_owned),
        after: v["after"].as_str().map(str::to_owned),
    })
}

/// A moment in a line for the agent: "jump 1 at frames 197 to 217 (3.3 s):
/// the original goes up 24 px; the model 23.1 px, on the same frame".
fn line(v: &Value) -> String {
    let what = match (v["kind"].as_str(), v["height"].as_f64()) {
        (Some("jump"), Some(h)) => format!("the original goes up {h} px"),
        (Some("counter" | "level"), _) => format!(
            "{} goes from {} to {}",
            v["part"].as_str().unwrap_or(""),
            v["before"].as_str().unwrap_or("?"),
            v["after"].as_str().unwrap_or("?")
        ),
        _ => "in the original".to_owned(),
    };
    let model = match &v["model"] {
        Value::Null => "the model doesn't".to_owned(),
        m => {
            let d = m["offset"].as_i64().unwrap_or(0);
            let when = match d {
                0 => "on the same frame".to_owned(),
                d if d > 0 => format!("{d} frames later"),
                d => format!("{} frames earlier", -d),
            };
            match m["height"].as_f64() {
                Some(h) => format!("the model {h} px, {when}"),
                None => format!("the model too, {when}"),
            }
        }
    };
    format!(
        "{} {} at frames {} to {} ({:.1} s): {what}; {model}",
        v["kind"].as_str().unwrap_or(""),
        v["n"],
        v["from"],
        v["to"],
        v["seconds_from"].as_f64().unwrap_or(0.0)
    )
}

pub fn moments(config: &Config, a: MomentsArgs) -> Result<Reply, String> {
    let dir = run_dir(config, &a.run)?;
    let recording = recording_of(&dir, a.recording.as_deref())?;
    let j = load(config, &dir, &recording)?;
    let all: Vec<Value> = j["moments"].as_array().cloned().unwrap_or_default();
    let list: Vec<&Value> = all
        .iter()
        .filter(|m| a.kind.as_deref().is_none_or(|k| m["kind"] == k))
        .collect();
    // What they said: a moment by its words, or "this" one at the frame.
    let found = a.said.as_deref().and_then(|said| {
        let ms: Vec<Moment> = list.iter().filter_map(|v| moment_of(v)).collect();
        let this = said
            .to_lowercase()
            .split_whitespace()
            .any(|w| w == "this" || w == "here");
        let pick = match (this, a.frame) {
            (true, Some(f)) => {
                let named = find(&ms, said).map(|m| m.kind);
                ms.iter()
                    .filter(|m| named.is_none_or(|k| m.kind == k))
                    .min_by_key(|m| {
                        if f >= m.from && f <= m.to {
                            0
                        } else {
                            m.from.abs_diff(f).min(m.to.abs_diff(f))
                        }
                    })
            }
            _ => find(&ms, said),
        }?;
        list.iter()
            .find(|v| v["kind"] == pick.kind && v["n"] == pick.n as u64)
            .map(|v| (*v).clone())
    });
    let lines: Vec<String> = list.iter().take(40).map(|v| line(v)).collect();
    Ok(Reply::ok(json!({
        "run": a.run,
        "recording": recording,
        "kinds": j["kinds"],
        "count": list.len(),
        "found": found.as_ref().map(|v| json!({"kind": v["kind"], "n": v["n"], "line": line(v), "moment": v})),
        "lines": lines,
        "more": list.len().saturating_sub(40),
    })))
}

pub fn refine_moment(config: &Config, a: RefineMomentArgs) -> Result<Reply, String> {
    if !KINDS.contains(&a.kind.as_str()) {
        return Err(format!(
            "a kind of moment is one of {}, not `{}`",
            KINDS.join(", "),
            a.kind
        ));
    }
    if a.n == 0 {
        return Err("moments are numbered from 1".into());
    }
    let dir = run_dir(config, &a.run)?;
    let recording = recording_of(&dir, a.recording.as_deref())?;
    let j = load(config, &dir, &recording)?;
    let m = j["moments"]
        .as_array()
        .and_then(|ms| {
            ms.iter()
                .find(|v| v["kind"] == a.kind.as_str() && v["n"] == a.n as u64)
        })
        .cloned()
        .ok_or_else(|| format!("the recording has no {} {}", a.kind, a.n))?;
    if m["mechanic"].is_null() {
        return Err(format!(
            "no mechanic owns {} moments, so there are no numbers to refine for it",
            a.kind
        ));
    }
    // The workspace starts it (POST /api/capture/{run}/refine with ask):
    // one loop a run at a time, shown beside the two games.
    let ask =
        json!({"recording": recording, "moment": {"kind": a.kind, "n": a.n}, "asked_by": "agent"});
    let file = dir.join("refine").join("ask.json");
    std::fs::create_dir_all(dir.join("refine")).map_err(|e| e.to_string())?;
    std::fs::write(&file, serde_json::to_vec_pretty(&ask).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    Ok(Reply::ok(json!({
        "asked": true,
        "moment": line(&m),
        "mechanic": m["mechanic"],
        "frames": m["focus"],
        "says": format!(
            "Asked to refine {} {}: the person's workspace starts it and shows it beside the two games, trying only {}'s numbers in frames {} to {}, then checking the value on every other {}.",
            a.kind, a.n, m["mechanic"].as_str().unwrap_or(""), m["focus"]["from"], m["focus"]["to"], a.kind
        ),
    })))
}
