//! Scenario tests (docs/host.md, "Scenarios"), run by `validate` and `gp host`: short scripted runs of a
//! model in the interpreter, each with expectations about the state, the
//! events and the final state hash. A template carries them in
//! `scenarios.toml`; a working copy inherits them on fork and adds its own
//! (`record`). They are what an agent runs after a change to see that the
//! game still does what it should.
//!
//! ```toml
//! format = "0.1.0"
//!
//! [scenarios.jump-apex]
//! description = "A held jump peaks 40 to 60 pixels up and lands again"
//! frames = 130
//! holds = ["a@50-60"]
//!
//! [[scenarios.jump-apex.expect]]
//! at = 100                       # the frame; default: the last
//! path = "entities.player[0].y"  # a state path; a pattern checks every match
//! op = "between"                 # eq ne lt le gt ge between
//! min = "100"
//! max = "200"
//!
//! [[scenarios.jump-apex.expect]]
//! event = "stomp"                # an event kind, counted over the run
//! count_min = 1
//!
//! [[scenarios.jump-apex.expect]]
//! state_hash = "00ab..."         # the interpreter's hash of the final state
//! ```
//!
//! A scenario gives `frames` and `holds` (as `run_preview` takes them) or a
//! `baseline` of the model's source. Runs are independent, so [`run_all`]
//! spreads them over threads; each loads its own interpreter.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::Config;
use crate::tools::{self, PreviewArgs};
use gp_interp::{Interp, Lifted};
use gp_model::Model;
use gp_runtime::Runtime;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The scenario file format's version.
pub const FORMAT: &str = "0.1.0";

/// The file a template or a working copy keeps its scenarios in.
pub const FILE: &str = "scenarios.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioFile {
    pub format: String,
    #[serde(default)]
    pub scenarios: BTreeMap<String, Scenario>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<String>,
    /// Instead of frames and holds: a baseline of the model's source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect: Vec<Expect>,
}

/// One expectation: a state value (`path` and `op`), an event count
/// (`event`) or the final state hash (`state_hash`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_min: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_max: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

const OPS: [&str; 7] = ["eq", "ne", "lt", "le", "gt", "ge", "between"];

impl ScenarioFile {
    pub fn parse(text: &str) -> Result<Self, String> {
        let f: ScenarioFile = toml::from_str(text).map_err(|e| format!("{FILE}: {e}"))?;
        if f.format != FORMAT {
            return Err(format!(
                "{FILE}: format {} isn't {FORMAT}, the one this build reads",
                f.format
            ));
        }
        for (name, s) in &f.scenarios {
            if !gp_model::path::is_name(&name.replace('-', "_")) {
                return Err(format!(
                    "{FILE}: `{name}` isn't a scenario name (letters, digits, `_`, `-`)"
                ));
            }
            s.check()
                .map_err(|e| format!("{FILE}: scenario `{name}`: {e}"))?;
        }
        Ok(f)
    }

    pub fn read(dir: &Path) -> Result<Option<Self>, String> {
        let path = dir.join(FILE);
        if !path.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        Self::parse(&text).map(Some)
    }
}

impl Scenario {
    fn check(&self) -> Result<(), String> {
        match (&self.baseline, self.frames, self.holds.is_empty()) {
            (Some(_), None, true) => {}
            (Some(_), _, _) => return Err("give a baseline, or frames and holds, not both".into()),
            (None, Some(_), _) => {}
            (None, None, _) => return Err("give `frames` (with `holds`), or a `baseline`".into()),
        }
        if self.expect.is_empty() {
            return Err("it expects nothing".into());
        }
        for (i, e) in self.expect.iter().enumerate() {
            e.check()
                .map_err(|m| format!("expectation {}: {m}", i + 1))?;
        }
        Ok(())
    }
}

impl Expect {
    fn check(&self) -> Result<(), String> {
        let kinds = [
            self.path.is_some(),
            self.event.is_some(),
            self.state_hash.is_some(),
        ];
        if kinds.iter().filter(|k| **k).count() != 1 {
            return Err("give exactly one of `path`, `event` and `state_hash`".into());
        }
        if self.path.is_some() {
            let op = self.op.as_deref().ok_or("a `path` needs an `op`")?;
            if !OPS.contains(&op) {
                return Err(format!("op `{op}` isn't one of {OPS:?}"));
            }
            if op == "between" {
                if self.min.is_none() || self.max.is_none() {
                    return Err("`between` needs `min` and `max`".into());
                }
            } else if self.value.is_none() {
                return Err(format!("`{op}` needs a `value`"));
            }
        }
        if self.event.is_some() && self.count_min.is_none() && self.count_max.is_none() {
            return Err("an `event` needs `count_min` or `count_max`".into());
        }
        Ok(())
    }

    fn describe(&self) -> String {
        let at = self.at.map_or(String::new(), |a| format!(" at frame {a}"));
        if let Some(p) = &self.path {
            let op = self.op.as_deref().unwrap_or("");
            let rhs = if op == "between" {
                format!(
                    "{}..{}",
                    self.min.as_deref().unwrap_or(""),
                    self.max.as_deref().unwrap_or("")
                )
            } else {
                self.value.clone().unwrap_or_default()
            };
            format!("{p} {op} {rhs}{at}")
        } else if let Some(e) = &self.event {
            format!(
                "event `{e}` count {}..{}",
                self.count_min.map_or(String::new(), |c| c.to_string()),
                self.count_max.map_or(String::new(), |c| c.to_string())
            )
        } else {
            "final state hash".to_owned()
        }
    }
}

/// One expectation's outcome.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub expect: String,
    pub ok: bool,
    pub got: Value,
}

/// A scenario's outcome. Holds nothing that varies between machines or
/// runs, so it can be a store object.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub name: String,
    pub ok: bool,
    pub frames: usize,
    pub state_hash: String,
    pub events: BTreeMap<String, u64>,
    pub checks: Vec<CheckResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn holds(op: &str, got: &Value, e: &Expect) -> Result<bool, String> {
    let want = |s: &Option<String>| s.clone().unwrap_or_default();
    if let Some(g) = number(got) {
        let n = |s: &Option<String>| {
            let t = want(s);
            t.parse::<f64>()
                .map_err(|_| format!("`{t}` isn't a number"))
        };
        return Ok(match op {
            "eq" => g == n(&e.value)?,
            "ne" => g != n(&e.value)?,
            "lt" => g < n(&e.value)?,
            "le" => g <= n(&e.value)?,
            "gt" => g > n(&e.value)?,
            "ge" => g >= n(&e.value)?,
            _ => g >= n(&e.min)? && g <= n(&e.max)?,
        });
    }
    // Flags and text: only equality means anything.
    let (g, w) = (text(got), want(&e.value));
    match op {
        "eq" => Ok(g == w),
        "ne" => Ok(g != w),
        _ => Err(format!("`{op}` needs a number, and the state holds {g}")),
    }
}

/// Everything one run needs, shared by the threads.
pub struct Env<'a> {
    pub config: &'a Config,
    pub model: &'a Model,
    pub lifted: &'a Lifted,
}

/// Runs one scenario.
pub fn run_one(env: &Env, name: &str, s: &Scenario) -> ScenarioResult {
    let mut r = ScenarioResult {
        name: name.to_owned(),
        ok: false,
        frames: 0,
        state_hash: String::new(),
        events: BTreeMap::new(),
        checks: Vec::new(),
        error: None,
    };
    if let Err(e) = run_into(env, s, &mut r) {
        r.error = Some(e);
        r.ok = false;
    }
    r
}

fn run_into(env: &Env, s: &Scenario, r: &mut ScenarioResult) -> Result<(), String> {
    let args = PreviewArgs {
        copy: String::new(),
        frames: s.frames,
        holds: s.holds.clone(),
        baseline: s.baseline.clone(),
        paths: Vec::new(),
        at: Vec::new(),
        picture: false,
    };
    let input = tools::input(env.config, env.model, &args)?;
    let last = input.len() as u64;
    let mut rt = Interp::load(env.model, env.lifted)?;
    // The values each frame's expectations read.
    let mut wanted: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    for e in &s.expect {
        if let Some(p) = &e.path {
            let at = e.at.unwrap_or(last);
            if at > last {
                return Err(format!(
                    "`{p}` is read at frame {at}, past the run's {last}"
                ));
            }
            wanted.entry(at).or_default().push(p.clone());
        }
    }
    let mut samples: BTreeMap<(u64, String), Value> = BTreeMap::new();
    let mut take = |rt: &Interp, at: u64| -> Result<(), String> {
        if let Some(paths) = wanted.get(&at) {
            let Value::Object(m) = tools::read_paths(rt, paths)? else {
                return Ok(());
            };
            for (k, v) in m {
                samples.insert((at, k), v);
            }
        }
        Ok(())
    };
    take(&rt, 0)?;
    for f in &input {
        for e in rt.step(std::slice::from_ref(f))? {
            if e.kind != gp_interp::runtime::FRAME_KIND {
                *r.events.entry(e.kind.clone()).or_default() += 1;
            }
        }
        take(&rt, rt.frame())?;
    }
    r.frames = input.len();
    r.state_hash = format!("{:016x}", rt.state_hash());
    for e in &s.expect {
        let mut c = CheckResult {
            expect: e.describe(),
            ok: false,
            got: Value::Null,
        };
        if let Some(p) = &e.path {
            let at = e.at.unwrap_or(last);
            // Every match of the path, as read.
            let mut got = serde_json::Map::new();
            for ((a, k), v) in &samples {
                if *a == at {
                    got.insert(k.clone(), v.clone());
                }
            }
            // Samples of other paths at the same frame belong to their own
            // expectations: keep the ones this path matches.
            let matched = rt
                .query(p)
                .map_err(|e| format!("`{p}`: {e}"))?
                .into_iter()
                .map(|x| x.path)
                .collect::<Vec<_>>();
            got.retain(|k, _| matched.contains(k));
            c.ok = !got.is_empty();
            for v in got.values() {
                c.ok &= holds(e.op.as_deref().unwrap_or(""), v, e)?;
            }
            c.got = if got.len() == 1 {
                got.into_iter()
                    .next()
                    .map(|(_, v)| v)
                    .unwrap_or(Value::Null)
            } else {
                Value::Object(got)
            };
        } else if let Some(k) = &e.event {
            let n = r.events.get(k).copied().unwrap_or(0);
            c.ok = e.count_min.is_none_or(|m| n >= m) && e.count_max.is_none_or(|m| n <= m);
            c.got = json!(n);
        } else if let Some(h) = &e.state_hash {
            c.ok = *h == r.state_hash;
            c.got = json!(r.state_hash);
        }
        r.checks.push(c);
    }
    r.ok = r.checks.iter().all(|c| c.ok);
    Ok(())
}

/// What a batch run was asked for.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// A working copy's name, or a template's id with `template: true`.
    pub copy: String,
    #[serde(default)]
    pub template: bool,
    /// Only these scenarios (default: all).
    #[serde(default)]
    pub names: Vec<String>,
    /// Threads (default: the machine's cores, at most 8).
    #[serde(default)]
    pub jobs: Option<usize>,
}

pub fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .min(8)
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Runs a directory's scenarios in parallel and stores the outcome. The
/// directory is a working copy or a template (it needs `model.toml` and
/// `scenarios.toml`).
pub fn run_all(config: &Config, dir: &Path, req: &Request) -> Result<Value, String> {
    let (model_text, model) = tools::load_model(dir)?;
    let file_text = std::fs::read_to_string(dir.join(FILE)).map_err(|_| {
        format!(
            "{} has no {FILE}; record one with `gp host record` or write it (docs/host.md)",
            dir.display()
        )
    })?;
    let file = ScenarioFile::parse(&file_text)?;
    for n in &req.names {
        if !file.scenarios.contains_key(n) {
            return Err(format!(
                "there's no scenario `{n}`; there are {:?}",
                file.scenarios.keys().collect::<Vec<_>>()
            ));
        }
    }
    let chosen: Vec<(&String, &Scenario)> = file
        .scenarios
        .iter()
        .filter(|(n, _)| req.names.is_empty() || req.names.contains(n))
        .collect();
    let assets = config
        .assets_of(dir, model.model.source.as_deref())
        .ok_or("the model names no source, so start with --assets")?;
    let lifted = Lifted::read_dir(&assets, &model)
        .map_err(|e| format!("{e} (lift the model's assets with gp assets lift)"))?;
    let env = Env {
        config,
        model: &model,
        lifted: &lifted,
    };
    let jobs = req
        .jobs
        .unwrap_or_else(default_jobs)
        .clamp(1, 64)
        .min(chosen.len().max(1));
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let done: std::sync::Mutex<Vec<(usize, ScenarioResult, u128)>> = Default::default();
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((name, s)) = chosen.get(i) else {
                        break;
                    };
                    let t = Instant::now();
                    let r = run_one(&env, name, s);
                    done.lock().unwrap_or_else(|e| e.into_inner()).push((
                        i,
                        r,
                        t.elapsed().as_millis(),
                    ));
                }
            });
        }
    });
    let wall = started.elapsed().as_millis();
    let mut done = done.into_inner().unwrap_or_else(|e| e.into_inner());
    done.sort_by_key(|(i, _, _)| *i);
    let serial: u128 = done.iter().map(|(_, _, ms)| ms).sum();
    let results: Vec<&ScenarioResult> = done.iter().map(|(_, r, _)| r).collect();
    let passed = results.iter().filter(|r| r.ok).count();

    // The outcome, without timings, is a content-addressed object: the same
    // model and scenarios give the same hash on any machine.
    let outcome = json!({
        "format": FORMAT,
        "model_sha256": sha(model_text.as_bytes()),
        "scenarios_sha256": sha(file_text.as_bytes()),
        "results": results,
    });
    let bytes = serde_json::to_vec_pretty(&outcome).expect("JSON serializes");
    let store = gp_evidence::Store::open(config.store()).map_err(|e| e.to_string())?;
    let object = store.put_bytes(&bytes).map_err(|e| e.to_string())?;
    Ok(json!({
        "ok": passed == results.len(),
        "dir": dir,
        "scenarios": results.len(),
        "passed": passed,
        "failed": results.len() - passed,
        "jobs": jobs,
        "store_object": object.to_hex(),
        "model_sha256": outcome["model_sha256"],
        "scenarios_sha256": outcome["scenarios_sha256"],
        "results": results,
        "millis": {
            "wall": wall,
            "serial": serial,
            "each": done.iter().map(|(_, r, ms)| (r.name.clone(), *ms)).collect::<BTreeMap<_, _>>(),
        },
    }))
}

/// What `record` is asked for.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub copy: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub frames: Option<usize>,
    #[serde(default)]
    pub holds: Vec<String>,
    #[serde(default)]
    pub baseline: Option<String>,
    /// State paths to pin (default: every `counters.` value and every
    /// state machine's `.state`).
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub overwrite: bool,
}

/// Runs a run on the copy's model and appends it to the copy's
/// `scenarios.toml` as a scenario that expects what happened: its final
/// state values at `paths`, its event counts and its state hash. This is how
/// a variant gets baselines of its own play (docs/host.md, "Variant
/// baselines"): the recorded scenario holds the variant to its own
/// behavior, and fails when a later edit changes it.
pub fn record(config: &Config, dir: &Path, rec: &Record) -> Result<Value, String> {
    if !gp_model::path::is_name(&rec.name.replace('-', "_")) {
        return Err(format!("`{}` isn't a scenario name", rec.name));
    }
    let (_, model) = tools::load_model(dir)?;
    let existing = ScenarioFile::read(dir)?;
    if !rec.overwrite
        && existing
            .as_ref()
            .is_some_and(|f| f.scenarios.contains_key(&rec.name))
    {
        return Err(format!(
            "the copy already has a scenario `{}`; pass overwrite",
            rec.name
        ));
    }
    let assets = config
        .assets_of(dir, model.model.source.as_deref())
        .ok_or("the model names no source, so start with --assets")?;
    let lifted = Lifted::read_dir(&assets, &model).map_err(|e| e.to_string())?;
    let scenario = Scenario {
        description: rec.description.clone(),
        frames: rec.frames,
        holds: rec.holds.clone(),
        baseline: rec.baseline.clone(),
        expect: vec![Expect {
            state_hash: Some("0".into()),
            ..Default::default()
        }],
    };
    scenario.check_shape()?;
    // Run once to learn what happened.
    let args = PreviewArgs {
        copy: String::new(),
        frames: rec.frames,
        holds: rec.holds.clone(),
        baseline: rec.baseline.clone(),
        paths: Vec::new(),
        at: Vec::new(),
        picture: false,
    };
    let input = tools::input(config, &model, &args)?;
    let mut rt = Interp::load(&model, &lifted)?;
    let mut events: BTreeMap<String, u64> = BTreeMap::new();
    for f in &input {
        for e in rt.step(std::slice::from_ref(f))? {
            if e.kind != gp_interp::runtime::FRAME_KIND {
                *events.entry(e.kind.clone()).or_default() += 1;
            }
        }
    }
    let mut expect = Vec::new();
    let values = if rec.paths.is_empty() {
        let mut m = serde_json::Map::new();
        for (p, v) in gp_interp::state::leaves(rt.spec(), rt.world()) {
            let name = gp_model::path::display(&p);
            if name.starts_with("counters.") || name.ends_with(".state") {
                m.insert(name, tools::value_json(&v));
            }
        }
        Value::Object(m)
    } else {
        tools::read_paths(&rt, &rec.paths)?
    };
    if let Value::Object(m) = &values {
        for (k, v) in m {
            expect.push(Expect {
                path: Some(k.clone()),
                op: Some("eq".into()),
                value: Some(text(v)),
                ..Default::default()
            });
        }
    }
    for (k, n) in &events {
        expect.push(Expect {
            event: Some(k.clone()),
            count_min: Some(*n),
            count_max: Some(*n),
            ..Default::default()
        });
    }
    expect.push(Expect {
        state_hash: Some(format!("{:016x}", rt.state_hash())),
        note: Some("recorded from this model's own play".into()),
        ..Default::default()
    });
    let scenario = Scenario { expect, ..scenario };
    // Appended as text, so the file's comments and other scenarios stay.
    let mut one = ScenarioFile {
        format: FORMAT.into(),
        scenarios: BTreeMap::new(),
    };
    one.scenarios.insert(rec.name.clone(), scenario);
    let block = toml::to_string(&one).map_err(|e| e.to_string())?;
    let path = dir.join(FILE);
    let out = match std::fs::read_to_string(&path) {
        Ok(old) => {
            if rec.overwrite && old.contains(&format!("[scenarios.{}]", rec.name)) {
                return Err(
                    "overwrite isn't supported on a file by appending: edit the file".into(),
                );
            }
            let body = block
                .strip_prefix(&format!("format = \"{FORMAT}\"\n"))
                .unwrap_or(&block)
                .trim_start_matches('\n');
            format!("{}\n\n{body}", old.trim_end())
        }
        Err(_) => block,
    };
    ScenarioFile::parse(&out)?;
    std::fs::write(&path, &out).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(json!({
        "ok": true,
        "written": path,
        "scenario": rec.name,
        "frames": input.len(),
        "state_hash": format!("{:016x}", rt.state_hash()),
        "pinned": values,
        "events": events,
    }))
}

impl Scenario {
    fn check_shape(&self) -> Result<(), String> {
        match (&self.baseline, self.frames, self.holds.is_empty()) {
            (Some(_), None, true) | (None, Some(_), _) => Ok(()),
            (Some(_), _, _) => Err("give a baseline, or frames and holds, not both".into()),
            (None, None, _) => Err("give `frames` (with `holds`), or a `baseline`".into()),
        }
    }
}
