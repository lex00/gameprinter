//! The tools `gp_commands` and `gp_run`: every gp command over MCP without
//! a second copy of any. The command tree comes from the binary serving
//! MCP (`gp --json self commands`, read from its own clap tree), so it is
//! never a hand-kept list; the rules for running each are
//! [`crate::policy`]'s table.
//!
//! gp_run checks a call's arguments against the command's own (an unknown
//! name is refused with the nearest ones), applies the rule, then runs the
//! same gp with `--json` in the games checkout, with the server's home,
//! under a time limit, and returns its exit code, stdout (as JSON when it
//! is JSON) and stderr. Long output is cut in the middle, keeping its start
//! and its end, and the whole of it saved.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::asks;
use crate::elicit;
use crate::held;
use crate::policy::{self, Decision, Road};
use crate::steps;
use crate::{Config, Reply, run_gp};

/// How long a command runs when the call doesn't say and the command
/// isn't one known to run long (`policy::LONG`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
/// The longest a call may ask for: a gate runs 30 to 60 minutes.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(3600);

/// The limit a call of `path` runs under when it gives no `timeout_s`.
pub fn default_timeout(path: &str) -> Duration {
    policy::long_timeout_s(path).map_or(DEFAULT_TIMEOUT, Duration::from_secs)
}

/// What a reply says of a command stopped at its time limit: that it
/// timed out and after how long, that it may have been fine and slow,
/// and how to ask for longer (or, at the most a call may ask, what else).
fn timed_out_words(path: &str, limit: Duration, asked: Option<u64>) -> String {
    let secs = limit.as_secs();
    let whose = match asked {
        Some(_) => "the timeout_s this call gave".to_owned(),
        None if policy::long_timeout_s(path).is_some() => {
            format!("gp_run's default for gp {path}, a command known to run long")
        }
        None => "gp_run's default for a command".to_owned(),
    };
    let longer = if limit < MAX_TIMEOUT {
        let more = (secs * 2).clamp(secs + 1, MAX_TIMEOUT.as_secs());
        format!(
            "To give it longer, call gp_run again with the same command and args and timeout_s set higher, {more} for example (at most {}).",
            MAX_TIMEOUT.as_secs()
        )
    } else {
        "That is the longest gp_run gives a command: narrow it with its flags (fewer clips, one level), or ask the person to run it in a terminal, where it has no limit.".to_owned()
    };
    format!(
        "Timed out: gp {path} was stopped after {secs} s, {whose}, with everything it started, before it finished. It may have been working and only slow (a debug build is slower), so this is not a failure of the command; what it printed so far is here. {longer}"
    )
}
/// The most stdout a reply carries whole. Past it the reply keeps the
/// start and the end ([`STDOUT_HEAD`], [`STDOUT_TAIL`]) and says it was cut,
/// and the whole of it is saved to a file.
pub const MAX_STDOUT: usize = 48 * 1024;
/// The most stderr a reply carries (its end, where errors are).
pub const MAX_STDERR: usize = 8 * 1024;
/// The most commands a search returns.
const MAX_FOUND: usize = 25;

/// One argument or flag of a command, as `gp self commands` prints it.
#[derive(Debug, Clone, Deserialize)]
pub struct ArgDef {
    pub name: String,
    #[serde(default)]
    pub long: Option<String>,
    #[serde(default)]
    pub short: Option<String>,
    #[serde(default)]
    pub position: Option<usize>,
    /// switch, count or value.
    pub kind: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub repeat: bool,
    #[serde(default)]
    pub values_per_flag: Option<usize>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub value_name: Option<String>,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub default: Vec<String>,
    #[serde(default)]
    pub help: String,
}

/// One command.
#[derive(Debug, Clone, Deserialize)]
pub struct CommandDef {
    pub path: String,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub args: Vec<ArgDef>,
    /// Takes the global --json.
    #[serde(default = "yes")]
    pub json: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupDef {
    pub path: String,
    #[serde(default)]
    pub about: String,
}

/// The command tree.
#[derive(Debug, Clone, Deserialize)]
pub struct Tree {
    pub commands: Vec<CommandDef>,
    #[serde(default)]
    pub groups: Vec<GroupDef>,
}

impl Tree {
    pub fn command(&self, path: &str) -> Option<&CommandDef> {
        self.commands.iter().find(|c| c.path == path)
    }

    pub fn group(&self, path: &str) -> Option<&GroupDef> {
        self.groups.iter().find(|g| g.path == path)
    }

    /// The commands under a group.
    fn under(&self, group: &str) -> Vec<&CommandDef> {
        let prefix = format!("{group} ");
        self.commands
            .iter()
            .filter(|c| c.path.starts_with(&prefix))
            .collect()
    }

    /// The paths nearest a word that isn't one, nearest first.
    fn nearest(&self, path: &str) -> Vec<String> {
        let all = self
            .commands
            .iter()
            .map(|c| c.path.as_str())
            .chain(self.groups.iter().map(|g| g.path.as_str()));
        nearest(path, all)
    }
}

/// The trees read so far, by the binary they came from.
static TREES: Mutex<BTreeMap<PathBuf, Arc<Tree>>> = Mutex::new(BTreeMap::new());

/// The command tree of the server's gp, read once.
pub fn tree(config: &Config) -> Result<Arc<Tree>, String> {
    let mut trees = TREES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(t) = trees.get(&config.gp) {
        return Ok(t.clone());
    }
    let (code, json) = run_gp(config, &["self".as_ref(), "commands".as_ref()])?;
    if code != 0 {
        return Err(format!("gp self commands exited {code}: {json}"));
    }
    let tree: Tree =
        serde_json::from_value(json).map_err(|e| format!("reading gp self commands' tree: {e}"))?;
    let tree = Arc::new(tree);
    trees.insert(config.gp.clone(), tree.clone());
    Ok(tree)
}

/// A path as an agent may write it: `gp corpus score`, `corpus  score`.
pub(crate) fn clean_path(p: &str) -> String {
    let words: Vec<&str> = p.split_whitespace().collect();
    let words = match words.first() {
        Some(&"gp") => &words[1..],
        _ => &words[..],
    };
    words.join(" ")
}

/// Edit distance, for the nearest names.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let here = row[j + 1];
            row[j + 1] = if ca == *cb {
                prev
            } else {
                1 + prev.min(row[j]).min(row[j + 1])
            };
            prev = here;
        }
    }
    row[b.len()]
}

/// Up to three of `names` nearest `word`: those that contain it or it
/// contains, then by edit distance, within a third of its length.
fn nearest<'a>(word: &str, names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut scored: Vec<(usize, &str)> = names
        .map(|n| {
            let d = if n.contains(word) || word.contains(n) {
                0
            } else {
                distance(word, n)
            };
            (d, n)
        })
        .filter(|(d, _)| *d <= (word.len() / 3).max(2))
        .collect();
    scored.sort();
    scored.dedup();
    scored
        .into_iter()
        .take(3)
        .map(|(_, n)| n.to_owned())
        .collect()
}

pub(crate) fn first_sentence(s: &str) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    let end = line
        .char_indices()
        .find(|&(i, c)| c == '.' && line[i + 1..].starts_with(' '))
        .map_or(line.len(), |(i, _)| i + 1);
    let mut out = line[..end].to_owned();
    if out.len() > 200 {
        let mut cut = 197;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("...");
    }
    out
}

fn row_of(c: &CommandDef) -> policy::Row {
    policy::row_or_guess(&c.path, &c.help)
}

/// A command as a list shows it.
fn entry(c: &CommandDef) -> Value {
    let row = row_of(c);
    let mut v = json!({
        "command": c.path,
        "about": first_sentence(&c.about),
        "road": row.road.name(),
        "rule": row.rule.name(),
    });
    if let Some(m) = row.road.mark() {
        v["mark"] = json!(m);
    }
    v
}

/// Commands sorted for a person's agent: the journey first, developer and
/// off-target tools last.
fn sorted(mut cs: Vec<&CommandDef>) -> Vec<&CommandDef> {
    cs.sort_by(|a, b| (row_of(a).road, &a.path).cmp(&(row_of(b).road, &b.path)));
    cs
}

/// A command's arguments as a JSON Schema: what gp_run's `args` takes.
pub fn args_schema(c: &CommandDef) -> Value {
    let mut props = Map::new();
    let mut required = Vec::new();
    for a in &c.args {
        let ty = match (a.kind.as_str(), a.ty.as_str()) {
            ("switch", _) => "boolean",
            ("count", _) => "integer",
            (_, "integer") => "integer",
            (_, "number") => "number",
            _ => "string",
        };
        let mut one = json!({ "type": ty });
        if !a.choices.is_empty() {
            one["enum"] = json!(a.choices);
        }
        let mut description = match a.position {
            Some(n) => format!(
                "Argument {n} ({}).",
                a.value_name.as_deref().unwrap_or(&a.name)
            ),
            None => format!("Flag --{}.", a.long.as_deref().unwrap_or(&a.name)),
        };
        if !a.help.is_empty() {
            description.push(' ');
            description.push_str(&a.help);
        }
        let mut prop = if a.repeat {
            json!({ "type": "array", "items": one, "description": description })
        } else {
            one["description"] = json!(description);
            one
        };
        if !a.default.is_empty() {
            prop["default"] = if a.repeat {
                json!(a.default)
            } else {
                json!(a.default[0])
            };
        }
        if a.required {
            required.push(a.name.clone());
        }
        props.insert(a.name.clone(), prop);
    }
    json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandsArgs {
    /// A group (`corpus`) to list its commands, or a command (`corpus
    /// score`) for its whole help, its rule and the JSON Schema of the
    /// `args` gp_run takes for it. Empty: the groups and the rules.
    #[serde(default)]
    pub path: Option<String>,
    /// Words to find in the commands' names and help (`sprite palette`):
    /// up to 25 commands that hold every word.
    #[serde(default)]
    pub find: Option<String>,
}

/// `gp_commands`.
pub fn commands(config: &Config, a: CommandsArgs) -> Result<Reply, String> {
    let tree = tree(config)?;
    commands_in(&tree, a)
}

pub fn commands_in(tree: &Tree, a: CommandsArgs) -> Result<Reply, String> {
    if let Some(words) = a.find.as_deref().filter(|w| !w.trim().is_empty()) {
        let words: Vec<String> = words.split_whitespace().map(str::to_lowercase).collect();
        let found: Vec<&CommandDef> = tree
            .commands
            .iter()
            .filter(|c| {
                let text = format!("{} {}", c.path, c.help).to_lowercase();
                words.iter().all(|w| text.contains(w.as_str()))
            })
            .collect();
        let total = found.len();
        let shown: Vec<Value> = sorted(found)
            .into_iter()
            .take(MAX_FOUND)
            .map(entry)
            .collect();
        return Ok(Reply::ok(json!({
            "find": words.join(" "),
            "found": total,
            "commands": shown,
            "next": "gp_commands with path set to one of these gives its arguments; gp_run runs it",
        })));
    }
    let path = a.path.as_deref().map(clean_path).unwrap_or_default();
    if path.is_empty() {
        return Ok(Reply::ok(overview(tree)));
    }
    if let Some(c) = tree.command(&path) {
        let row = row_of(c);
        let mut v = json!({
            "command": c.path,
            "help": c.help,
            "road": row.road.name(),
            "rule": row.rule.name(),
            "rule_in_words": row.rule.words(),
            "args": args_schema(c),
            "call": {
                "tool": "gp_run",
                "arguments": {
                    "command": c.path,
                    "args": c.args.iter().filter(|a| a.required).map(|a| (a.name.clone(), json!(format!("<{}>", a.value_name.as_deref().unwrap_or(&a.name))))).collect::<Map<_, _>>(),
                },
            },
        });
        if let Some(m) = row.road.mark() {
            v["mark"] = json!(m);
        }
        v["timeout_s"] = json!(default_timeout(&c.path).as_secs());
        if row.laya != policy::Laya::No {
            v["laya"] = json!(
                "it may ask Laya, the decision server on 127.0.0.1:7720, for moves; it runs only while Laya or gp serve already answers, since a tool never starts a server"
            );
        }
        let subs = tree.under(&c.path);
        if !subs.is_empty() {
            v["subcommands"] = json!(sorted(subs).into_iter().map(entry).collect::<Vec<_>>());
        }
        return Ok(Reply::ok(v));
    }
    let under = tree.under(&path);
    if !under.is_empty() {
        let about = tree
            .group(&path)
            .map(|g| g.about.clone())
            .unwrap_or_default();
        return Ok(Reply::ok(json!({
            "group": path,
            "about": about,
            "commands": sorted(under).into_iter().map(entry).collect::<Vec<_>>(),
            "next": "gp_commands with path set to one of these gives its arguments; gp_run runs it",
        })));
    }
    Ok(Reply::refused(json!({
        "error": format!("gp has no command or group `{path}`"),
        "nearest": tree.nearest(&path),
    })))
}

/// The top level: each group or top-level command with its road and how
/// many commands it holds, journey first; and the rules.
fn overview(tree: &Tree) -> Value {
    let mut tops: BTreeMap<&str, (Road, usize, String)> = BTreeMap::new();
    for c in &tree.commands {
        let top = c.path.split(' ').next().unwrap_or("");
        let road = row_of(c).road;
        let about = if c.path == top {
            c.about.clone()
        } else {
            tree.group(top).map(|g| g.about.clone()).unwrap_or_default()
        };
        let e = tops.entry(top).or_insert((road, 0, about));
        e.0 = e.0.min(road);
        e.1 += 1;
    }
    let mut list: Vec<(&str, (Road, usize, String))> = tops.into_iter().collect();
    list.sort_by(|a, b| (a.1.0, a.0).cmp(&(b.1.0, b.0)));
    let groups: Vec<Value> = list
        .into_iter()
        .map(|(top, (road, n, about))| {
            let mut v = json!({
                "path": top,
                "about": first_sentence(&about),
                "commands": n,
                "road": road.name(),
            });
            if let Some(m) = road.mark() {
                v["mark"] = json!(m);
            }
            v
        })
        .collect();
    json!({
        "commands": tree.commands.len(),
        "groups": groups,
        "rules": policy::summary(),
        "roads": {
            "journey": "on a person's road: porting a game, a theme, a new game",
            "port stage": "a stage of the port the tools run without a person",
            "developer": "maintenance and research tools, listed last: don't steer a person to them",
            "off target": "kept for now, likely to go: don't steer a person to them",
        },
        "next": "gp_commands with path set to a group lists its commands with their rules; with a command's path it gives its help and the args gp_run takes; find searches the help",
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunArgs {
    /// The command's words after gp: `corpus score`, `roms scan`.
    pub command: String,
    /// Its arguments and flags by name, as gp_commands gives them for the
    /// command: `{"game": "rooftop", "dry-run": true, "clip": ["a", "b"]}`.
    /// A switch takes true, a repeated one a list.
    #[serde(default)]
    pub args: Map<String, Value>,
    /// Set this only after the person said yes, in this conversation, to
    /// this exact action (publishing, landing, spending, a big download).
    /// Never set it on your own judgment. While the person's page is open
    /// (gp serve runs for this home) it isn't enough: their YES on the
    /// page's card is, and this call runs only once they pressed it.
    #[serde(default)]
    pub confirmed_by_person: bool,
    /// The ask a refusal gave (ask_id), when calling again after the
    /// person answered it on the page. The same command with the same
    /// arguments finds its answer without it too.
    #[serde(default)]
    pub ask_id: Option<String>,
    /// The id of the loop's step this call is (an open step from next or
    /// gp loop steps, `s` and 12 hex digits). When the home's loop.toml has
    /// steps_required on, a call that writes, lands or publishes runs only
    /// citing an open step whose commands include its command; reads never
    /// need one. A worker the runner started cites its own step unasked.
    #[serde(default)]
    pub step: Option<String>,
    /// Seconds before the command is stopped: at most 3600. Left out, 300,
    /// or for a command known to run long (corpus score, model refine,
    /// the gates) the longer default gp_commands shows as timeout_s.
    #[serde(default)]
    pub timeout_s: Option<u64>,
}

/// A call's arguments as gp's words, checked against the command's own.
#[derive(Debug, Default, PartialEq)]
pub struct Built {
    pub argv: Vec<String>,
    /// Each value given, by its argument's name.
    pub values: Vec<(String, String)>,
    /// The names given (a switch only when true).
    pub given: BTreeSet<String>,
}

fn key(k: &str) -> String {
    k.trim_start_matches('-').replace('_', "-")
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Checks `args` against `c`'s arguments and builds gp's words: options
/// first, then the positionals in order.
pub fn build(c: &CommandDef, args: &Map<String, Value>) -> Result<Built, String> {
    let mut by_name: BTreeMap<String, Value> = BTreeMap::new();
    let mut unknown = Vec::new();
    for (k, v) in args {
        let k = key(k);
        let def = c.args.iter().find(|a| {
            a.name == k || a.long.as_deref() == Some(&k) || a.short.as_deref() == Some(&k)
        });
        match def {
            Some(d) => {
                by_name.insert(d.name.clone(), v.clone());
            }
            None => unknown.push(k),
        }
    }
    if !unknown.is_empty() {
        let names = c.args.iter().map(|a| a.name.as_str());
        let near: Vec<String> = unknown
            .iter()
            .map(|u| {
                let n = nearest(u, names.clone());
                if n.is_empty() {
                    format!("`{u}`")
                } else {
                    format!("`{u}` (nearest: {})", n.join(", "))
                }
            })
            .collect();
        let all: Vec<&str> = c.args.iter().map(|a| a.name.as_str()).collect();
        return Err(format!(
            "gp {} has no {}; it takes: {}",
            c.path,
            near.join(", "),
            if all.is_empty() {
                "nothing".to_owned()
            } else {
                all.join(", ")
            }
        ));
    }
    let mut out = Built::default();
    let mut positionals: Vec<(usize, Vec<String>)> = Vec::new();
    for d in &c.args {
        let Some(v) = by_name.get(&d.name).filter(|v| !v.is_null()) else {
            if d.required {
                return Err(format!(
                    "gp {} needs `{}`: {}",
                    c.path,
                    d.name,
                    first_sentence(&d.help)
                ));
            }
            continue;
        };
        let flag = match (&d.long, &d.short) {
            (Some(l), _) => format!("--{l}"),
            (None, Some(s)) => format!("-{s}"),
            (None, None) => String::new(),
        };
        match d.kind.as_str() {
            "switch" => match v {
                Value::Bool(true) => {
                    out.argv.push(flag);
                    out.given.insert(d.name.clone());
                }
                Value::Bool(false) => {}
                _ => return Err(format!("`{}` is a switch: true or false", d.name)),
            },
            "count" => {
                let n = match v {
                    Value::Bool(b) => u64::from(*b),
                    Value::Number(n) => n
                        .as_u64()
                        .ok_or_else(|| format!("`{}` counts: a whole number from 0", d.name))?,
                    _ => return Err(format!("`{}` counts: a whole number from 0", d.name)),
                };
                for _ in 0..n.min(16) {
                    out.argv.push(flag.clone());
                }
                if n > 0 {
                    out.given.insert(d.name.clone());
                }
            }
            _ => {
                let vals: Vec<String> = match v {
                    Value::Array(items) if d.repeat => items
                        .iter()
                        .map(|i| {
                            scalar(i)
                                .ok_or_else(|| format!("`{}` takes a list of plain values", d.name))
                        })
                        .collect::<Result<_, _>>()?,
                    Value::Array(_) => {
                        return Err(format!("`{}` takes one value, not a list", d.name));
                    }
                    other => vec![scalar(other).ok_or_else(|| {
                        format!("`{}` takes a plain value, not an object", d.name)
                    })?],
                };
                for s in &vals {
                    if !d.choices.is_empty() && !d.choices.contains(s) {
                        return Err(format!(
                            "`{}` is one of {}; got {s:?}",
                            d.name,
                            d.choices.join(", ")
                        ));
                    }
                    if d.ty == "integer" && s.parse::<i128>().is_err() {
                        return Err(format!("`{}` is a whole number; got {s:?}", d.name));
                    }
                    if d.ty == "number" && s.parse::<f64>().is_err() {
                        return Err(format!("`{}` is a number; got {s:?}", d.name));
                    }
                    out.values.push((d.name.clone(), s.clone()));
                }
                if vals.is_empty() {
                    continue;
                }
                out.given.insert(d.name.clone());
                match d.position {
                    Some(p) => positionals.push((p, vals)),
                    None if d.values_per_flag.is_some() => {
                        out.argv.push(flag);
                        out.argv.extend(vals);
                    }
                    None => {
                        for s in vals {
                            if s.starts_with('-') && d.long.is_some() {
                                out.argv.push(format!("{flag}={s}"));
                            } else {
                                out.argv.push(flag.clone());
                                out.argv.push(s);
                            }
                        }
                    }
                }
            }
        }
    }
    positionals.sort_by_key(|(p, _)| *p);
    let words: Vec<String> = positionals.into_iter().flat_map(|(_, v)| v).collect();
    if words.iter().any(|w| w.starts_with('-')) {
        out.argv.push("--".to_owned());
    }
    out.argv.extend(words);
    Ok(out)
}

/// What a child process did.
#[derive(Debug)]
pub struct Ran {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    pub seconds: f64,
}

/// Runs `cmd` until it ends or `limit` passes; then stops it and every
/// process it started (its own process group).
pub fn run_limited(mut cmd: Command, limit: Duration) -> Result<Ran, String> {
    crate::proc::own_group(&mut cmd);
    let started = Instant::now();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("starting gp: {e}"))?;
    let reader = |mut r: Box<dyn std::io::Read + Send>| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            let _ = tx.send(buf);
        });
        rx
    };
    let out = reader(Box::new(child.stdout.take().expect("piped")));
    let err = reader(Box::new(child.stderr.take().expect("piped")));
    let mut timed_out = false;
    let status = loop {
        match child
            .try_wait()
            .map_err(|e| format!("waiting for gp: {e}"))?
        {
            Some(s) => break Some(s),
            None if started.elapsed() >= limit => {
                timed_out = true;
                crate::proc::stop_group(child.id(), Duration::from_millis(200));
                let _ = child.kill();
                break child.wait().ok();
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    // A grandchild that outlived it may hold the pipes; don't wait on it.
    let wait = Duration::from_secs(5);
    Ok(Ran {
        code: if timed_out {
            None
        } else {
            status.and_then(|s| s.code())
        },
        stdout: out.recv_timeout(wait).unwrap_or_default(),
        stderr: err.recv_timeout(wait).unwrap_or_default(),
        timed_out,
        seconds: started.elapsed().as_secs_f64(),
    })
}

/// The port of the address in $`var` when it is on 127.0.0.1 (as gp
/// serve reads $GP_LAYA_URL and a tool $GP_SERVE_URL), else `default`.
fn local_port(var: &str, default: u16) -> u16 {
    std::env::var(var)
        .ok()
        .and_then(|u| {
            u.trim_end_matches('/')
                .strip_prefix("http://127.0.0.1:")?
                .parse()
                .ok()
        })
        .unwrap_or(default)
}

/// Whether something answers on 127.0.0.1:`port`.
fn answers(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(500),
    )
    .is_ok()
}

/// Where the whole of a cut output is kept.
fn saved_dir(config: &Config) -> PathBuf {
    config.work.join("gp-run")
}

fn save(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("making {}: {e}", dir.display()))?;
    let file = dir.join(name);
    std::fs::write(&file, bytes).map_err(|e| format!("writing {}: {e}", file.display()))?;
    Ok(file)
}

fn cut_at(s: &str, mut n: usize) -> &str {
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

/// The last `n` bytes of `s` or fewer, from a character's start.
fn last(s: &str, n: usize) -> &str {
    let mut from = s.len().saturating_sub(n);
    while !s.is_char_boundary(from) {
        from += 1;
    }
    &s[from..]
}

/// Of a cut stdout, how much of its start the reply carries...
pub const STDOUT_HEAD: usize = 32 * 1024;
/// ...and of its end, where a command prints its result.
pub const STDOUT_TAIL: usize = 12 * 1024;
/// The most a cut JSON reply's small top-level values take, whole.
const SMALL_VALUES: usize = 4 * 1024;
/// A top-level value this size or smaller is small.
const SMALL_VALUE: usize = 1024;

/// The keys of a JSON object with how many bytes each holds, for a reply
/// too large to carry whole.
fn key_sizes(v: &Value) -> Value {
    match v {
        Value::Object(o) => json!(
            o.iter()
                .map(|(k, v)| (k.clone(), json!(v.to_string().len())))
                .collect::<Map<_, _>>()
        ),
        Value::Array(a) => json!({ "items": a.len() }),
        _ => Value::Null,
    }
}

/// A JSON object's small top-level values, whole (`ok`, counts, a
/// summary), smallest first until [`SMALL_VALUES`] bytes.
fn small_values(v: &Value) -> Option<Map<String, Value>> {
    let o = v.as_object()?;
    let mut sized: Vec<(usize, &String, &Value)> = o
        .iter()
        .map(|(k, v)| (v.to_string().len(), k, v))
        .filter(|(n, _, _)| *n <= SMALL_VALUE)
        .collect();
    sized.sort_by_key(|(n, k, _)| (*n, (*k).clone()));
    let mut out = Map::new();
    let mut used = 0;
    for (n, k, v) in sized {
        if used + n > SMALL_VALUES {
            break;
        }
        used += n;
        out.insert(k.clone(), v.clone());
    }
    Some(out)
}

/// The output's part of a reply: stdout as JSON or text, stderr, and
/// what was cut and where it is whole; and stdout parsed, when it is JSON.
/// A stdout past [`MAX_STDOUT`] is cut in the middle: the reply keeps its
/// first [`STDOUT_HEAD`] and last [`STDOUT_TAIL`] bytes (and for a JSON
/// object, its small top-level values whole), says so in `cut`, and the
/// whole of it is saved to a file.
pub fn output(
    ran: &Ran,
    dir: &Path,
    stem: &str,
) -> Result<(Map<String, Value>, Option<Value>), String> {
    let mut o = Map::new();
    let stdout = String::from_utf8_lossy(&ran.stdout);
    let parsed: Option<Value> = serde_json::from_str(stdout.trim()).ok();
    if stdout.len() <= MAX_STDOUT {
        match &parsed {
            Some(v) => o.insert("json".into(), v.clone()),
            None => o.insert("stdout".into(), json!(stdout)),
        };
    } else {
        let ext = if parsed.is_some() { "json" } else { "txt" };
        let file = save(dir, &format!("{stem}.stdout.{ext}"), &ran.stdout)?;
        let head = cut_at(&stdout, STDOUT_HEAD);
        let tail = last(&stdout, STDOUT_TAIL);
        let left_out = stdout.len() - head.len() - tail.len();
        o.insert("stdout".into(), json!(head));
        o.insert("stdout_end".into(), json!(tail));
        let mut cut = json!({
            "stdout_bytes": stdout.len(),
            "stdout_shown_bytes": head.len() + tail.len(),
            "stdout_left_out_bytes": left_out,
            "stdout_saved": file,
            "note": format!(
                "stdout was cut: it was {} bytes, more than a reply carries ({MAX_STDOUT}), so stdout holds its first {} bytes and stdout_end its last {}, and the {left_out} bytes between are left out{}. The whole of it is in the file named in stdout_saved: read that file for the rest, or narrow the command with its flags for a shorter answer.",
                stdout.len(),
                head.len(),
                tail.len(),
                if parsed.is_some() {
                    " (neither part is JSON on its own; json_small has the small top-level values whole, json_keys every key's size)"
                } else {
                    ""
                },
            ),
        });
        if let Some(v) = &parsed {
            cut["json_keys"] = key_sizes(v);
            if let Some(small) = small_values(v) {
                o.insert("json_small".into(), json!(small));
            }
        }
        o.insert("cut".into(), cut);
    }
    let stderr = String::from_utf8_lossy(&ran.stderr);
    let stderr = stderr.trim_end();
    if stderr.len() <= MAX_STDERR {
        if !stderr.is_empty() {
            o.insert("stderr".into(), json!(stderr));
        }
    } else {
        let file = save(dir, &format!("{stem}.stderr.txt"), &ran.stderr)?;
        o.insert("stderr".into(), json!(last(stderr, MAX_STDERR)));
        let cut = o
            .entry("cut")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("an object");
        cut.insert("stderr_shown_bytes".into(), json!(MAX_STDERR));
        cut.insert("stderr_bytes".into(), json!(stderr.len()));
        cut.insert("stderr_saved".into(), json!(file));
        cut.insert(
            "stderr_note".into(),
            json!(format!(
                "stderr was cut: it was {} bytes, so only its last {MAX_STDERR} are here; the whole of it is in the file named in stderr_saved",
                stderr.len()
            )),
        );
    }
    Ok((o, parsed))
}

/// What gp said when a command couldn't run, in its own words: the JSON
/// it printed's `error`, else its error line on stderr (`gp: WHY`, which
/// gp prints last; `gp COMMAND: WHY` for a refusal before it ran), else
/// stderr's last line. Whole, never cut or shortened.
pub fn gp_said(ran: &Ran, parsed: Option<&Value>) -> Option<String> {
    if let Some(e) = parsed.and_then(|v| v["error"].as_str()) {
        return Some(e.to_owned());
    }
    let stderr = String::from_utf8_lossy(&ran.stderr);
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if let Some(l) = lines.iter().rev().find_map(|l| l.strip_prefix("gp: ")) {
        return Some(l.trim().to_owned());
    }
    lines
        .iter()
        .rev()
        .find(|l| l.starts_with("gp ") && l.contains(": "))
        .or(lines.last())
        .map(|l| (*l).to_owned())
}

/// The keys under which commands report what they wrote.
const WROTE_KEYS: &[&str] = &[
    "wrote", "written", "writes", "out", "output", "file", "files", "dir", "folder", "path",
    "paths", "into", "session", "copy",
];

/// The person's yes for a call that needs one: Ok with a note when it may
/// run (a yes recorded on the page, now used; or, with no page served for
/// this home, the agent's confirmed_by_person as before), else the refusal.
/// A refusal with nothing asked yet writes the ask the page shows.
fn the_yes(
    config: &Config,
    c: &CommandDef,
    path: &str,
    built: &Built,
    ask: policy::Ask,
    row: &policy::Row,
    a: &RunArgs,
) -> Result<String, Value> {
    let home = &config.home;
    let k = asks::key(path, &built.argv);
    let served = asks::page_served(home);
    if let Some(v) = a.ask_id.as_deref().and_then(|id| asks::read(home, id))
        && v["key"] == json!(k)
        && v["ran"].is_object()
    {
        return Err(json!({
            "refused": format!(
                "the loop's runner ran this on the person's yes (ask {}), so it doesn't run again: it is {}. Its last lines: {}",
                v["id"].as_str().unwrap_or(""),
                v["ran"]["state"].as_str().unwrap_or("running"),
                v["ran"]["lines"].as_array().map(|l| l.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" | ")).unwrap_or_default()
            ),
            "ran": v["ran"],
            "ask_id": v["id"],
            "rule": row.rule.name(),
        }));
    }
    let found = asks::find(home, a.ask_id.as_deref(), &k);
    // What a call that replaces data would change, from its plan, so the
    // card shows it and a yes holds only while the plan is the same.
    let plan = (ask == policy::Ask::Replace)
        .then(|| plan_of(config, c, path, &built.argv))
        .flatten();
    let refusal = |words: String, ask_v: Option<&Value>| {
        let mut v = json!({
            "refused": words,
            "rule": row.rule.name(),
            "needs": ask.name(),
            "page_open": served,
        });
        if let Some(p) = &plan {
            v["plan"] = p["says"].clone();
        }
        if let Some(x) = ask_v {
            v["ask_id"] = x["id"].clone();
            v["ask"] = json!({"says": x["says"], "state": x["state"]});
        }
        if let policy::Rule::Ask(_, free) = row.rule
            && !free.is_empty()
        {
            v["runs_at_once_with"] = json!(free);
        }
        v
    };
    let by_form = |v: &Value| match form_yes(home, path, v) {
        FormYes::Yes(note) => Ok(note),
        FormYes::Refused(words) => Err(refusal(words, Some(v))),
        FormYes::NoForm(_) if a.confirmed_by_person => Ok(ON_YOUR_WORD.to_owned()),
        FormYes::NoForm(why) => Err(refusal(
            format!(
                "gp {path} needs the person's yes first: {}. The form asking them couldn't be shown ({why}), and no page is open for this home (gp serve isn't running), so there is no card: if they say yes to this exact action, call again with confirmed_by_person: true. The ask is kept (ask_id), so if they open the page they can answer it there.",
                ask.before()
            ),
            Some(v),
        )),
    };
    let found = match found {
        // A yes to a plan that has changed since is no yes to this one.
        asks::Found::Yes(v)
            if v.get("plan").is_some()
                && plan.as_ref().map(|p| &p["hash"]) != Some(&v["plan"]["hash"]) =>
        {
            let _ = v;
            asks::Found::Other(json!({}))
        }
        f => f,
    };
    match found {
        asks::Found::Yes(v) => Ok(format!(
            "the person said yes to this on the page (ask {}); that yes is used now, so another run needs another yes{}",
            v["id"].as_str().unwrap_or(""),
            said_beside(&v)
        )),
        asks::Found::No(v) => Err(refusal(
            match v["words"].as_str() {
                Some(w) => format!(
                    "gp {path}: the person said no to this, and wrote: \"{w}\". Don't run it; change what they said, then raise a new card."
                ),
                None => format!(
                    "gp {path}: the person said no to this (on the page, or in the form their client showed). Don't run it; ask them what they would like instead."
                ),
            },
            Some(&v),
        )),
        asks::Found::Open(v) if served => Err(refusal(
            format!(
                "gp {path} waits for the person's yes: it is on their page as a card with YES and NO. Tell them in one sentence what it does and that the card is waiting. Then call next with wait_s until waits_for_yes shows this ask (ask_id) answered, and call gp_run again with ask_id. Setting confirmed_by_person doesn't replace their answer while the page is open."
            ),
            Some(&v),
        )),
        // No page, and a client that shows forms: the person answers the
        // ask's own words in a form, whatever the agent set.
        asks::Found::Open(v) if elicit::can_ask() => by_form(&v),
        found if !served && a.confirmed_by_person && !elicit::can_ask() => {
            let _ = found;
            Ok(ON_YOUR_WORD.to_owned())
        }
        found => {
            // A landing's card names the clip that shows it.
            if path == "model gate"
                && let Some(why) =
                    asks::gate_card_refused(&built.values, |n| built.given.contains(n))
            {
                return Err(refusal(format!("gp {path}: {why}."), None));
            }
            let other = matches!(found, asks::Found::Other(_));
            let cost = (ask == policy::Ask::Spend && c.args.iter().any(|d| d.name == "dry-run"))
                .then(|| dry_run_cost(config, c, path, &built.argv))
                .flatten();
            let made = asks::make(
                home,
                &asks::New {
                    command: path,
                    argv: &built.argv,
                    values: &built.values,
                    about: &c.about,
                    needs: ask,
                    cost,
                    plan: plan.clone(),
                    held: None,
                    step: a.step.as_deref(),
                },
            );
            let lead = if other {
                "The ask named was for another command or other arguments, or what it would change is no longer what the person said yes to; a changed argument or plan needs a new yes. "
            } else {
                ""
            };
            let words = if served {
                format!(
                    "{lead}gp {path} needs the person's yes first: {}. It is on their page now as a card with YES and NO. Tell them in one sentence what it does and that the card is waiting; then call next with wait_s until waits_for_yes shows this ask (ask_id) answered, and call gp_run again with ask_id.",
                    ask.before()
                )
            } else {
                format!(
                    "{lead}gp {path} needs the person's yes first: {}. No page is open for this home (gp serve isn't running), so there is no card: if they say yes to this exact action, call again with confirmed_by_person: true. The ask is kept (ask_id), so if they open the page they can answer it there.",
                    ask.before()
                )
            };
            match made {
                Ok(v) if !served && elicit::can_ask() => by_form(&v),
                Ok(v) => Err(refusal(words, Some(&v))),
                Err(e) => Err(refusal(
                    format!("{words} (The ask couldn't be written: {e})"),
                    None,
                )),
            }
        }
    }
}

/// The note on a call that ran with no page open on the agent's
/// confirmed_by_person.
const ON_YOUR_WORD: &str = "no page is open for this home (gp serve isn't running), so this ran on your word that the person said yes to this exact action, as before; with the page open, their YES on the page's card is needed";

/// What the person did with the form asking their yes to ask `v`, with no
/// page open (questions-as-elicitation).
#[derive(Debug, PartialEq)]
pub enum FormYes {
    /// They said yes: the yes is recorded and used now; the note says so.
    Yes(String),
    /// They said no or closed it: the refusal's words.
    Refused(String),
    /// No form could be shown (why).
    NoForm(String),
}

/// The person's words beside their answer to ask `v`, as the end of a
/// sentence, or nothing when they wrote none.
fn said_beside(v: &Value) -> String {
    v["words"]
        .as_str()
        .map(|w| format!(". Beside their yes they wrote: \"{w}\""))
        .unwrap_or_default()
}

/// Asks the person's yes to ask `v` (gp `path`) in a form: its card's own
/// words and one switch, the same whichever agent asks. A yes is written to
/// the ask as the page's YES would be, and used at once; a no, or a form
/// sent with the switch off, is the page's NO; a form closed unanswered
/// leaves the ask open.
pub fn form_yes(home: &Path, path: &str, v: &Value) -> FormYes {
    let id = v["id"].as_str().unwrap_or("");
    let (message, schema) = elicit::yes_form(v["says"].as_str().unwrap_or(""));
    match elicit::ask(&message, schema) {
        elicit::Answer::Sent(f) if elicit::said_yes(&f) => {
            if let Err(e) = asks::answer(home, id, asks::Answer::Yes) {
                return FormYes::Refused(format!(
                    "gp {path}: the person said yes in the form, but it couldn't be written to the ask: {e}"
                ));
            }
            match asks::find(home, Some(id), v["key"].as_str().unwrap_or("")) {
                asks::Found::Yes(_) => FormYes::Yes(format!(
                    "the person said yes to this in the form their client showed (ask {id}); that yes is used now, so another run needs another yes"
                )),
                _ => FormYes::Refused(format!(
                    "gp {path}: the person said yes in the form, but the yes couldn't be used (ask {id})"
                )),
            }
        }
        elicit::Answer::Sent(_) | elicit::Answer::Declined => {
            let _ = asks::answer(home, id, asks::Answer::No);
            FormYes::Refused(format!(
                "gp {path}: the person said no to this in the form their client showed. Don't run it; ask them what they would like instead."
            ))
        }
        elicit::Answer::Closed => FormYes::Refused(format!(
            "gp {path}: the person closed the form asking their yes without answering, so it didn't run. The ask is kept (ask_id); when they want it, call gp_run again with the same command and args, and the form asks them again."
        )),
        elicit::Answer::NoForm(why) => FormYes::NoForm(why),
    }
}

/// What a dry run says the command would cost, when it says (theme
/// generate prints dollars); none when it can't run quickly.
fn dry_run_cost(config: &Config, c: &CommandDef, path: &str, argv: &[String]) -> Option<String> {
    let mut words = vec!["--dry-run".to_owned()];
    // The dry-run flag before any `--` that ends the options.
    words.extend(argv.iter().filter(|x| *x != "--yes").cloned());
    asks::cost_of(&run_quietly(config, c, path, &words)?)
}

/// A call's plan: the same call without `--write` (or with `--dry-run`
/// where the command has no `write`), which writes nothing, and what its
/// JSON says it would change or remove of data already in the person's
/// games (`replaces`, each with `says`), with a hash of that: `{says,
/// hash}`. None when the command has no plan form or the plan can't run.
pub fn plan_of(config: &Config, c: &CommandDef, path: &str, argv: &[String]) -> Option<Value> {
    let has = |n: &str| c.args.iter().any(|d| d.name == n && d.kind == "switch");
    let words: Vec<String> = if has("write") {
        argv.iter().filter(|x| *x != "--write").cloned().collect()
    } else if has("dry-run") {
        std::iter::once("--dry-run".to_owned())
            .chain(argv.iter().cloned())
            .collect()
    } else {
        return None;
    };
    let v = run_quietly(config, c, path, &words)?;
    let replaces = v.get("replaces")?.as_array()?;
    let says: Vec<Value> = replaces
        .iter()
        .filter_map(|r| r["says"].as_str().map(|s| json!(s)))
        .collect();
    let hash = asks::key(&serde_json::to_string(replaces).ok()?, &[]);
    Some(json!({"says": says, "hash": hash}))
}

/// The games checkout `gp PATH` runs in: a loop worker's own worktree of
/// it, when it has one, so what the command writes stays there; else the
/// person's. A gate always runs in the person's, the checkout it lands on.
pub(crate) fn games_for<'a>(config: &'a Config, path: &str) -> Option<&'a Path> {
    match &config.worker_games {
        Some(w) if !path.ends_with(" gate") => Some(w),
        _ => config.games.as_deref(),
    }
}

/// Runs `gp PATH WORDS` as gp_run would, for at most two minutes, and
/// gives its JSON.
fn run_quietly(config: &Config, c: &CommandDef, path: &str, words: &[String]) -> Option<Value> {
    let mut cmd = Command::new(&config.gp);
    cmd.env(crate::CALLER_ENV, "mcp");
    cmd.env("GP_NO_HINTS", "1");
    if c.json {
        cmd.arg("--json");
    }
    cmd.args(path.split(' ')).args(words);
    cmd.env("GP_HOME", &config.home);
    match games_for(config, path) {
        Some(g) => {
            cmd.env("GP_GAMES_DIR", g).current_dir(g);
        }
        None => {
            cmd.current_dir(&config.work);
        }
    }
    let ran = run_limited(cmd, Duration::from_secs(120)).ok()?;
    serde_json::from_slice(&ran.stdout).ok()
}

/// The person's answer to a change the net held for this exact call, when
/// there is one: a yes (or, with no page open, the agent's
/// confirmed_by_person on an open one) puts the held change in place
/// without running the command again; an open one or a no refuses. None:
/// run the call.
fn held_answer(config: &Config, path: &str, built: &Built, a: &RunArgs) -> Option<Reply> {
    let home = &config.home;
    let served = asks::page_served(home);
    let hk = asks::held_key(path, &built.argv);
    let put = |v: &Value, how: &str| match held::apply(&v["held"]) {
        Ok(files) => Reply::ok(json!({
            "command": format!("gp {path}"),
            "applied": files,
            "yes": format!("{how} (ask {}); the change gp {path} made and gp_run held is in place now, without running the command again. Another change needs another yes", v["id"].as_str().unwrap_or("")),
        })),
        Err(e) => Reply::refused(json!({
            "refused": format!("gp {path}: the person's yes was for a held change that can't go in: {e}"),
            "ask_id": v["id"],
        })),
    };
    match asks::find(home, a.ask_id.as_deref(), &hk) {
        asks::Found::Yes(v) => Some(put(&v, "the person said yes to this change on the page")),
        asks::Found::No(v) => Some(Reply::refused(json!({
            "refused": format!("gp {path}: the person said no on the page to the change this call makes to what is already in their games. Don't run it again; ask them what they would like instead."),
            "ask_id": v["id"],
        }))),
        // No page, and a client that shows forms: the person answers the
        // held change's card in a form.
        asks::Found::Open(v) if !served && elicit::can_ask() => match form_yes(home, path, &v) {
            FormYes::Yes(_) => Some(put(
                &v,
                "the person said yes to this change in the form their client showed",
            )),
            FormYes::Refused(words) => Some(Reply::refused(json!({
                "refused": words,
                "ask_id": v["id"],
                "would_change": held::each_in_words(&v["held"]),
            }))),
            FormYes::NoForm(_) if a.confirmed_by_person => match asks::mark_used(home, &v) {
                Ok(()) => Some(put(
                    &v,
                    "no page is open for this home and no form could be shown, so this went in on your word that the person said yes to this change after you showed it to them",
                )),
                Err(e) => Some(Reply::error(e)),
            },
            FormYes::NoForm(why) => Some(Reply::refused(json!({
                "refused": format!("gp {path}'s change to what is already in the person's games waits for their yes, and the form asking them couldn't be shown ({why}). No page is open for this home: show them what would change (the held change's diff, in {}), and if they say yes to it here, call again with confirmed_by_person: true.", v["held"]["dir"].as_str().unwrap_or("?")),
                "ask_id": v["id"],
                "would_change": held::each_in_words(&v["held"]),
                "diff": held::diff_shown(&v["held"]),
                "page_open": served,
            }))),
        },
        asks::Found::Open(v) if !served && a.confirmed_by_person => match asks::mark_used(home, &v)
        {
            Ok(()) => Some(put(
                &v,
                "no page is open for this home, so this went in on your word that the person said yes to this change after you showed it to them",
            )),
            Err(e) => Some(Reply::error(e)),
        },
        asks::Found::Open(v) => Some(Reply::refused(json!({
            "refused": if served {
                format!("gp {path}'s change to what is already in the person's games waits for their yes: it is on their page as a card with YES and NO. Tell them what it changes, then call next with wait_s until waits_for_yes shows this ask (ask_id) answered, and call gp_run again with ask_id. Setting confirmed_by_person doesn't replace their answer while the page is open.")
            } else {
                format!("gp {path}'s change to what is already in the person's games waits for their yes. No page is open for this home: show them what would change (the held change's diff, in {}), and if they say yes to it here, call again with confirmed_by_person: true.", v["held"]["dir"].as_str().unwrap_or("?"))
            },
            "ask_id": v["id"],
            "would_change": held::each_in_words(&v["held"]),
            "diff": held::diff_shown(&v["held"]),
            "page_open": served,
        }))),
        _ => None,
    }
}

/// The reply's words for a change the net caught and held, with the ask
/// it made.
fn held_refusal(
    config: &Config,
    c: &CommandDef,
    path: &str,
    built: &Built,
    shown: &str,
    h: Value,
) -> Map<String, Value> {
    let served = asks::page_served(&config.home);
    let each = held::each_in_words(&h);
    let says = format!(
        "Let {shown} change what is already in your games: {}. It made these changes; they were put back as they were and are held until you say yes.",
        each.join("; ")
    );
    let made = asks::make(
        &config.home,
        &asks::New {
            command: path,
            argv: &built.argv,
            values: &built.values,
            about: &c.about,
            needs: policy::Ask::Replace,
            cost: None,
            plan: None,
            held: Some((h.clone(), says)),
            step: None,
        },
    );
    let then = if served {
        "Tell the person in plain words what would change (would_change, diff) and that it waits on their page as a card with YES and NO; then call next with wait_s until waits_for_yes shows this ask (ask_id) answered, and call gp_run again with ask_id (or the same command and args): a yes puts the held change in place without running the command again."
    } else if elicit::can_ask() {
        "No page is open for this home (gp serve isn't running), so there is no card, but the person's client shows forms: call gp_run again with the same command and args, and a form asks them, in the card's words, whether the held change goes in; their yes puts it in place without running the command again."
    } else {
        "No page is open for this home (gp serve isn't running), so there is no card: show the person what would change (would_change, diff), and if they say yes to that change here, call gp_run again with the same command and args and confirmed_by_person: true, which puts the held change in place without running the command again."
    };
    let mut o = Map::new();
    o.insert(
        "refused".into(),
        json!(format!(
            "gp {path} changed or removed files already in the games checkout, which a tool may not do before the person has seen the change: {}. gp_run put them back as they were and holds the change (held_in). Files it made and lines it only added stay. {then}",
            each.join("; ")
        )),
    );
    o.insert("needs".into(), json!(policy::Ask::Replace.name()));
    o.insert("would_change".into(), json!(each));
    o.insert("diff".into(), json!(held::diff_shown(&h)));
    o.insert("held_in".into(), h["dir"].clone());
    o.insert("page_open".into(), json!(served));
    match made {
        Ok(v) => {
            o.insert("ask_id".into(), v["id"].clone());
        }
        Err(e) => {
            o.insert(
                "ask_error".into(),
                json!(format!("the ask couldn't be written: {e}")),
            );
        }
    }
    o
}

/// The loop's check (docs/loop.md, policy::decide_step): with the home's
/// steps_required on, or in a worker the runner started (`GP_STEP`), a
/// call that writes, lands or publishes runs only as an open step that
/// runs its command. None when it may go on; else the refusal.
fn loop_check(
    config: &Config,
    path: &str,
    decision: &Decision,
    cited: Option<&str>,
    row: &policy::Row,
) -> Option<Value> {
    // A test's server isn't the worker whose gate runs the tests.
    let bound = std::env::var(steps::STEP_ENV)
        .ok()
        .filter(|s| !cfg!(test) && !s.trim().is_empty());
    if !steps::settings(&config.home).steps_required && bound.is_none() {
        return None;
    }
    if !matches!(
        decision,
        Decision::Run { writes: true } | Decision::RunHeld | Decision::NeedsYes(_)
    ) {
        return None;
    }
    let cur = match steps::current(config) {
        Ok(v) => v,
        Err(e) => {
            return Some(json!({
                "refused": format!("gp {path} writes, lands or publishes, and the loop's steps couldn't be read to check it against them: {e}"),
                "needs": "step",
                "rule": row.rule.name(),
            }));
        }
    };
    let open = cur["open"].as_array().cloned().unwrap_or_default();
    let held = steps::held(&config.home);
    let halted = cur["halted"].as_object().map(|h| {
        format!(
            "step {} was marked wrong: {}",
            h.get("id").and_then(Value::as_str).unwrap_or("?"),
            h.get("why")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim_end_matches('.')
        )
    });
    let st = policy::Steps {
        cited,
        open: &open,
        held: &held,
        halted: halted.as_deref(),
        bound: bound.as_deref(),
    };
    match policy::decide_step(decision.clone(), path, &st) {
        Decision::Refused(why) => Some(json!({
            "refused": why,
            "needs": "step",
            "rule": row.rule.name(),
            "open_steps": open.iter().map(|s| json!({
                "id": s["id"], "who": s["who"], "subject": s["subject"],
                "says": s["says"], "commands": s["commands"],
            })).collect::<Vec<_>>(),
        })),
        _ => None,
    }
}

/// `gp_run`.
pub fn run(config: &Config, a: RunArgs) -> Result<Reply, String> {
    let tree = tree(config)?;
    run_in(config, &tree, a, &answers)
}

/// gp_run on a tree, with `answers` telling whether a port is served.
pub fn run_in(
    config: &Config,
    tree: &Tree,
    a: RunArgs,
    answers: &dyn Fn(u16) -> bool,
) -> Result<Reply, String> {
    let path = clean_path(&a.command);
    let Some(c) = tree.command(&path) else {
        let under = tree.under(&path);
        if !under.is_empty() {
            return Ok(Reply::refused(json!({
                "error": format!("`{path}` is a group; run one of its commands"),
                "commands": sorted(under).into_iter().map(entry).collect::<Vec<_>>(),
            })));
        }
        return Ok(Reply::refused(json!({
            "error": format!("gp has no command `{path}`"),
            "nearest": tree.nearest(&path),
        })));
    };
    let built = match build(c, &a.args) {
        Ok(b) => b,
        Err(e) => {
            return Ok(Reply::refused(json!({
                "error": e,
                "next": format!("gp_commands with path \"{path}\" lists its arguments"),
            })));
        }
    };
    let row = row_of(c);
    let given = |n: &str| built.given.contains(n);
    let mut yes_note: Option<String> = None;
    let decision = policy::decide(&row, &given, &built.values);
    if let Some(r) = loop_check(config, &path, &decision, a.step.as_deref(), &row) {
        return Ok(Reply::refused(r));
    }
    // A row the person is still iterating on isn't gated by its worker, dry
    // run or card, until they press LAND IT on it (crate::iterating).
    let step = a.step.clone().filter(|s| !s.trim().is_empty()).or_else(|| {
        std::env::var(steps::STEP_ENV)
            .ok()
            .filter(|s| !cfg!(test) && !s.trim().is_empty())
    });
    if let Some(why) =
        crate::iterating::refused(&config.home, &path, step.as_deref(), &built.values)
    {
        return Ok(Reply::refused(json!({
            "refused": why,
            "needs": "land it",
            "rule": row.rule.name(),
        })));
    }
    if !path.starts_with("loop ")
        && matches!(
            decision,
            Decision::Run { writes: true } | Decision::RunHeld | Decision::NeedsYes(_)
        )
        && let Some(why) = steps::no_runner_here(&config.home)
    {
        return Ok(Reply::refused(json!({
            "refused": steps::no_runner_says(&why),
            "needs": "runner",
            "rule": row.rule.name(),
        })));
    }
    match decision.clone() {
        Decision::Refused(why) => {
            return Ok(Reply::refused(json!({
                "refused": format!("gp {path} isn't run from a tool: {why}"),
                "rule": row.rule.name(),
            })));
        }
        Decision::NeedsYes(ask) => match the_yes(config, c, &path, &built, ask, &row, &a) {
            Ok(n) => yes_note = Some(n),
            Err(refused) => return Ok(Reply::refused(refused)),
        },
        _ => {}
    }
    let (laya, serve) = (
        local_port("GP_LAYA_URL", 7720),
        local_port("GP_SERVE_URL", 7710),
    );
    if policy::may_start_laya(&row, &given) && !answers(laya) && !answers(serve) {
        return Ok(Reply::refused(json!({
            "refused": format!(
                "gp {path} asks Laya, the decision server, for moves, and with neither Laya (127.0.0.1:{laya}) nor gp serve (127.0.0.1:{serve}) answering it would start a server on port {laya}, which a tool never does. Ask the person to start gp serve, then call again."
            ),
            "needs": "laya",
            "rule": row.rule.name(),
        })));
    }
    let stem = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis()),
        path.replace(' ', "-")
    );
    // The net: a write's change to what is already in the games checkout
    // waits for the person's yes (crate::held). A command run in a loop
    // worker's own worktree leaves the person's checkout alone, and its
    // changes reach it only through a gate.
    let games = games_for(config, &path);
    let mut snap: Option<(held::Snap, PathBuf)> = None;
    if decision == Decision::RunHeld
        && games == config.games.as_deref()
        && let Some(top) = games.and_then(held::top_of)
    {
        if let Some(r) = held_answer(config, &path, &built, &a) {
            return Ok(r);
        }
        let dir = saved_dir(config).join(format!("snap-{stem}"));
        match held::snapshot(&top, &dir) {
            Ok(s) => snap = Some((s, dir)),
            Err(e) => {
                return Ok(Reply::refused(json!({
                    "refused": format!("gp {path} wasn't run: {e}"),
                    "rule": row.rule.name(),
                })));
            }
        }
    }
    let limit = a
        .timeout_s
        .map_or_else(|| default_timeout(&path), Duration::from_secs)
        .min(MAX_TIMEOUT);
    let mut cmd = Command::new(&config.gp);
    cmd.env(crate::CALLER_ENV, "mcp");
    if c.json {
        cmd.arg("--json");
    }
    cmd.args(path.split(' ')).args(&built.argv);
    cmd.env("GP_HOME", &config.home);
    // The addresses checked above, so the command asks Laya where gp_run
    // looked for it (a --decide left out reads $GP_LAYA_URL) and finds gp
    // serve there too: one variable decides everywhere.
    cmd.env("GP_LAYA_URL", format!("http://127.0.0.1:{laya}"));
    cmd.env("GP_SERVE_URL", format!("http://127.0.0.1:{serve}"));
    match games {
        Some(g) => {
            cmd.env("GP_GAMES_DIR", g).current_dir(g);
        }
        None => {
            cmd.current_dir(&config.work);
        }
    }
    // The tool's own commands (gp tool gate) act on gameprinter's checkout,
    // found from the current folder: run them there, not in the games
    // checkout, whose commits the tool gate took for its base.
    if path.starts_with("tool ") {
        cmd.current_dir(&config.repo);
    }
    let shown = std::iter::once("gp".to_owned())
        .chain(c.json.then(|| "--json".to_owned()))
        .chain(path.split(' ').map(str::to_owned))
        .chain(built.argv.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let ran = run_limited(cmd, limit)?;
    let mut reply = json!({
        "command": shown,
        "rule": row.rule.name(),
        "exit": ran.code,
        "seconds": (ran.seconds * 10.0).round() / 10.0,
    });
    let o = reply.as_object_mut().expect("an object");
    let (out, parsed) = output(&ran, &saved_dir(config), &stem)?;
    for (k, v) in out {
        o.insert(k, v);
    }
    let writes = matches!(decision, Decision::Run { writes: true } | Decision::RunHeld)
        || yes_note.is_some();
    if let Some(n) = &yes_note {
        o.insert("yes".into(), json!(n));
    }
    if writes {
        // Read from the whole output, so a cut one still says what it wrote.
        let wrote: Map<String, Value> = parsed
            .as_ref()
            .and_then(Value::as_object)
            .map(|j| {
                j.iter()
                    .filter(|(k, _)| WROTE_KEYS.contains(&k.as_str()))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default();
        o.insert(
            "writes".into(),
            json!(if wrote.is_empty() {
                "this command may write the person's games, copies or home; its output says what it did"
            } else {
                "this command writes the person's games, copies or home; wrote has what it reported"
            }),
        );
        if !wrote.is_empty() {
            o.insert("wrote".into(), json!(wrote));
        }
    }
    if let Some(m) = row.road.mark() {
        o.insert("mark".into(), json!(m));
    }
    if let Some((s, dir)) = &snap {
        let caught = held::changes(s).and_then(|list| {
            if list.is_empty() {
                return Ok(None);
            }
            let to = config.home.join("work").join("held").join(&stem);
            held::hold(s, &list, &shown, &to).map(Some)
        });
        let _ = std::fs::remove_dir_all(dir);
        match caught {
            Ok(None) => {}
            Ok(Some(h)) => {
                for (k, v) in held_refusal(config, c, &path, &built, &shown, h) {
                    o.insert(k, v);
                }
                return Ok(Reply::refused(reply));
            }
            Err(e) => {
                o.insert(
                    "net".into(),
                    json!(format!(
                        "gp_run couldn't check what this call changed in the games checkout: {e}. Look at git status there before going on."
                    )),
                );
                return Ok(Reply::refused(reply));
            }
        }
    }
    if ran.timed_out {
        o.insert(
            "timed_out".into(),
            json!(timed_out_words(&path, limit, a.timeout_s)),
        );
        o.insert("timeout_s".into(), json!(limit.as_secs()));
        return Ok(Reply::refused(reply));
    }
    // Exit 1 is a check that ran and failed, which isn't an error; exit 2
    // (or none) is a command that couldn't run.
    let failed = ran.code != Some(0) && ran.code != Some(1);
    Ok(if failed {
        // gp's own words for why it couldn't run, first, so nothing else in
        // the reply (its next step, a hint) reads as the reason.
        if let Some(e) = gp_said(&ran, parsed.as_ref()) {
            o.insert("error".into(), json!(e));
        }
        Reply::refused(reply)
    } else {
        Reply::ok(reply)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Tree {
        serde_json::from_value(json!({
            "commands": [
                {
                    "path": "model gate",
                    "about": "Hold a patch to a game in the games repo to its corpus.",
                    "help": "Hold a patch to a game in the games repo to its corpus: PASS or FAIL, and what passes lands",
                    "args": [
                        {"name": "game", "position": 1, "kind": "value", "type": "path", "required": true, "value_name": "GAME", "help": "The game"},
                        {"name": "patch", "long": "patch", "kind": "value", "type": "path", "help": "The patch"},
                        {"name": "dry-run", "long": "dry-run", "kind": "switch", "type": "boolean", "help": "Land nothing"},
                        {"name": "removes", "long": "removes", "kind": "value", "type": "string", "repeat": true},
                        {"name": "jobs-n", "long": "jobs-n", "kind": "value", "type": "integer"},
                        {"name": "mode", "long": "mode", "kind": "value", "type": "string", "choices": ["fast", "full"]},
                        {"name": "verbose", "short": "v", "kind": "count", "type": "boolean"}
                    ]
                },
                {"path": "env", "about": "Show resolved paths.", "help": "Show resolved paths", "args": []},
                {"path": "serve", "about": "The page.", "help": "The page", "args": []},
                {"path": "registry publish", "about": "Publish.", "help": "Publish a folder", "args": [
                    {"name": "target", "position": 1, "kind": "value", "type": "string", "required": true}
                ]},
                {"path": "mcp", "about": "Serve MCP.", "help": "Serve", "args": [], "json": false}
            ],
            "groups": [{"path": "model", "about": "Models."}, {"path": "registry", "about": "Registries."}]
        }))
        .unwrap()
    }

    /// Two writes, for the net.
    fn net_tree() -> Tree {
        serde_json::from_value(json!({
            "commands": [
                {"path": "model fmt", "about": "Format a model.", "help": "Format a model", "args": [
                    {"name": "game", "position": 1, "kind": "value", "type": "string", "required": true}
                ]},
                {"path": "corpus import", "about": "Import clips.", "help": "Import clips", "args": [
                    {"name": "game", "position": 1, "kind": "value", "type": "string", "required": true},
                    {"name": "write", "long": "write", "kind": "switch", "type": "boolean"},
                    {"name": "replace", "long": "replace", "kind": "value", "type": "string", "repeat": true}
                ]}
            ],
            "groups": [{"path": "model", "about": "Models."}, {"path": "corpus", "about": "Corpora."}]
        }))
        .unwrap()
    }

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn arguments_become_gp_s_words() {
        let t = tree();
        let gate = t.command("model gate").unwrap();
        let b = build(
            gate,
            &args(json!({
                "game": "rooftop",
                "dry_run": true,
                "--patch": "-weird.diff",
                "removes": ["a", "b*"],
                "v": 2,
                "jobs-n": 3,
            })),
        )
        .unwrap();
        assert_eq!(
            b.argv,
            [
                "--patch=-weird.diff",
                "--dry-run",
                "--removes",
                "a",
                "--removes",
                "b*",
                "--jobs-n",
                "3",
                "-v",
                "-v",
                "rooftop"
            ]
        );
        assert!(b.given.contains("dry-run"));
        let b = build(gate, &args(json!({"game": "-odd"}))).unwrap();
        assert_eq!(b.argv, ["--", "-odd"]);
    }

    #[test]
    fn a_wrong_argument_is_refused_with_the_nearest_names() {
        let t = tree();
        let gate = t.command("model gate").unwrap();
        let e = build(gate, &args(json!({"game": "g", "dryrun": true}))).unwrap_err();
        assert!(e.contains("`dryrun` (nearest: dry-run)"), "{e}");
        let e = build(gate, &args(json!({}))).unwrap_err();
        assert!(e.contains("needs `game`"), "{e}");
        let e = build(gate, &args(json!({"game": "g", "dry-run": "yes"}))).unwrap_err();
        assert!(e.contains("switch"), "{e}");
        let e = build(gate, &args(json!({"game": ["a", "b"]}))).unwrap_err();
        assert!(e.contains("one value"), "{e}");
        let e = build(gate, &args(json!({"game": "g", "jobs-n": "many"}))).unwrap_err();
        assert!(e.contains("whole number"), "{e}");
        let e = build(gate, &args(json!({"game": "g", "mode": "slow"}))).unwrap_err();
        assert!(e.contains("fast, full"), "{e}");
    }

    fn config(dir: &Path, gp: &Path) -> Config {
        Config {
            gp: gp.to_owned(),
            repo: dir.to_owned(),
            work: dir.join("work"),
            home: dir.join("home"),
            rom: None,
            assets: None,
            games: None,
            worker_games: None,
        }
    }

    #[test]
    fn a_loop_workers_commands_run_in_its_worktree_and_its_gates_on_the_persons_checkout() {
        let d = Path::new("/x");
        let mut c = config(d, d);
        c.games = Some(d.join("games"));
        assert_eq!(
            games_for(&c, "corpus import"),
            Some(d.join("games").as_path())
        );
        c.worker_games = Some(d.join("tree"));
        assert_eq!(
            games_for(&c, "corpus import"),
            Some(d.join("tree").as_path())
        );
        assert_eq!(
            games_for(&c, "mechanic probe"),
            Some(d.join("tree").as_path())
        );
        assert_eq!(games_for(&c, "model gate"), Some(d.join("games").as_path()));
        assert_eq!(games_for(&c, "tool gate"), Some(d.join("games").as_path()));
    }

    /// A stand-in gp: a shell script that runs `body`.
    fn fake_gp(dir: &Path, body: &str) -> PathBuf {
        let f = dir.join("gp");
        std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
        crate::proc::make_executable(&f).unwrap();
        std::fs::create_dir_all(dir.join("work")).unwrap();
        f
    }

    fn call(c: &Config, v: Value) -> Reply {
        let a: RunArgs = serde_json::from_value(v).unwrap();
        run_in(c, &tree(), a, &|_| false).unwrap()
    }

    /// With the home's steps_required on, a write runs only citing an open
    /// step that runs it; a read runs without one (docs/loop.md).
    #[test]
    fn with_steps_required_a_write_cites_an_open_step() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(
            dir.path(),
            r#"if [ "$2" = loop ]; then echo '{"open": [{"id": "s111111111111", "who": "agent", "subject": "g", "says": "Format it.", "commands": ["model fmt"]}], "halted": null}'; else echo '{"ok": true}'; fi"#,
        );
        let c = config(dir.path(), &gp);
        let t = net_tree();
        let a = |step: Option<&str>| RunArgs {
            command: "model fmt".into(),
            args: serde_json::from_value(json!({"game": "g"})).unwrap(),
            confirmed_by_person: false,
            ask_id: None,
            step: step.map(str::to_owned),
            timeout_s: None,
        };
        // Off (the default): runs as before.
        let r = run_in(&c, &t, a(None), &|_| false).unwrap();
        assert!(!r.is_error, "{}", r.json);
        let mut s = steps::settings(&c.home);
        s.steps_required = true;
        steps::save(&c.home, &s).unwrap();
        let r = run_in(&c, &t, a(None), &|_| false).unwrap();
        assert!(r.is_error);
        assert_eq!(r.json["needs"], "step");
        let why = r.json["refused"].as_str().unwrap();
        assert!(why.starts_with("gp model fmt writes, lands or publishes, so gp_run runs it only as a step the loop gave"), "{why}");
        assert!(
            why.contains("s111111111111 (agent, g): Format it, runs model fmt"),
            "{why}"
        );
        assert_eq!(r.json["open_steps"][0]["id"], "s111111111111");
        let r = run_in(&c, &t, a(Some("s111111111111")), &|_| false).unwrap();
        assert!(!r.is_error, "{}", r.json);
        let r = run_in(&c, &t, a(Some("s000000000000")), &|_| false).unwrap();
        assert!(
            r.json["refused"]
                .as_str()
                .unwrap()
                .contains("isn't open now")
        );
        // A read needs no step.
        let r = call(&c, json!({"command": "env"}));
        assert!(!r.is_error, "{}", r.json);
    }

    #[test]
    fn a_publish_is_refused_until_the_person_says_yes() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(
            dir.path(),
            "echo '{\"published\": true, \"files\": [\"a\"]}'",
        );
        let c = config(dir.path(), &gp);
        let r = call(
            &c,
            json!({"command": "registry publish", "args": {"target": "/tmp/r"}}),
        );
        assert!(r.is_error);
        let why = r.json["refused"].as_str().unwrap();
        assert!(why.contains("needs the person's yes"), "{why}");
        let r = call(
            &c,
            json!({"command": "gp registry publish", "args": {"target": "/tmp/r"}, "confirmed_by_person": true}),
        );
        assert!(!r.is_error, "{}", r.json);
        assert_eq!(r.json["json"]["published"], true);
        assert_eq!(r.json["wrote"]["files"], json!(["a"]));
        assert_eq!(r.json["command"], "gp --json registry publish /tmp/r");
    }

    /// gp serve runs for the test's home: this process stands for it.
    fn serve_page(c: &Config) {
        crate::asks::mark_page_served(&c.home, 0).unwrap();
    }

    fn answer(c: &Config, id: &Value, yes: bool) {
        let a = if yes {
            crate::asks::Answer::Yes
        } else {
            crate::asks::Answer::No
        };
        crate::asks::answer(&c.home, id.as_str().unwrap(), a).unwrap();
    }

    #[test]
    fn with_the_page_open_a_publish_runs_once_on_the_person_s_yes() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(dir.path(), "echo '{\"published\": true}'");
        let c = config(dir.path(), &gp);
        serve_page(&c);
        let publish = json!({"command": "registry publish", "args": {"target": "/tmp/r"}, "confirmed_by_person": true});
        let r = call(&c, publish.clone());
        assert!(
            r.is_error,
            "confirmed_by_person isn't enough with the page open"
        );
        let id = r.json["ask_id"].clone();
        assert!(id.is_string(), "{}", r.json);
        assert!(r.json["ask"]["says"].as_str().unwrap().contains("/tmp/r"));
        // Asked again before the answer: the same ask, still waiting.
        let r = call(&c, publish.clone());
        assert!(r.is_error);
        assert_eq!(r.json["ask_id"], id);
        assert!(
            r.json["refused"]
                .as_str()
                .unwrap()
                .contains("waits for the person's yes")
        );
        assert_eq!(crate::asks::pending(&c.home).len(), 1);
        answer(&c, &id, true);
        let r = call(
            &c,
            json!({"command": "registry publish", "args": {"target": "/tmp/r"}, "ask_id": id}),
        );
        assert!(!r.is_error, "{}", r.json);
        assert_eq!(r.json["json"]["published"], true);
        assert!(r.json["yes"].as_str().unwrap().contains("used"));
        // The yes is used: the next run asks again.
        let r = call(&c, publish);
        assert!(r.is_error);
        assert_ne!(r.json["ask_id"], id);
    }

    fn call_net(c: &Config, v: Value) -> Reply {
        let a: RunArgs = serde_json::from_value(v).unwrap();
        run_in(c, &net_tree(), a, &|_| false).unwrap()
    }

    /// A games checkout in git with one committed file, for the net.
    fn games(c: &mut Config, dir: &Path) -> PathBuf {
        let top = dir.join("games");
        std::fs::create_dir_all(top.join("g")).unwrap();
        let g = |args: &[&str]| {
            let o = Command::new("git")
                .arg("-C")
                .arg(&top)
                .args(args)
                .output()
                .unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        };
        g(&["init", "-q"]);
        g(&["config", "user.email", "t@example.com"]);
        g(&["config", "user.name", "t"]);
        g(&["config", "core.hooksPath", "/dev/null"]);
        std::fs::write(top.join("g/corpus.toml"), "a\nb\nc\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-q", "-m", "base"]);
        c.games = Some(top.clone());
        top
    }

    /// A write that changes a file already in the games checkout is put
    /// back and held, and the held change goes in on the person's word,
    /// without running the command again; lines it only adds, and new
    /// files, stay at once.
    #[test]
    fn a_write_that_changes_what_is_in_the_games_is_held_until_the_person_says_yes() {
        let dir = tempfile::tempdir().unwrap();
        // Each run of the stand-in counts itself in ran.txt, outside the
        // checkout.
        let gp = fake_gp(
            dir.path(),
            &format!(
                "echo x >> {}/ran.txt\ncase \"$*\" in\n*add*) printf 'd\\n' >> g/corpus.toml ;;\n*) printf 'a\\nc\\n' > g/corpus.toml; echo n > g/new.toml ;;\nesac\necho '{{\"ok\": true}}'",
                dir.path().display()
            ),
        );
        let mut c = config(dir.path(), &gp);
        let top = games(&mut c, dir.path());
        let ran = || {
            std::fs::read_to_string(dir.path().join("ran.txt"))
                .unwrap()
                .lines()
                .count()
        };
        let fmt = json!({"command": "model fmt", "args": {"game": "g"}});
        let r = call_net(&c, fmt.clone());
        assert!(r.is_error, "{}", r.json);
        let why = r.json["refused"].as_str().unwrap();
        assert!(
            why.starts_with("gp model fmt changed or removed files already in the games checkout, which a tool may not do before the person has seen the change: g/corpus.toml (1 line removed or changed, 0 added)"),
            "{why}"
        );
        assert!(why.contains("confirmed_by_person: true"), "{why}");
        assert_eq!(r.json["needs"], "replaces");
        assert!(
            r.json["diff"].as_str().unwrap().contains("-b"),
            "{}",
            r.json
        );
        assert_eq!(
            std::fs::read_to_string(top.join("g/corpus.toml")).unwrap(),
            "a\nb\nc\n"
        );
        assert!(top.join("g/new.toml").exists(), "a new file stays");
        let id = r.json["ask_id"].clone();
        let ask = crate::asks::read(&c.home, id.as_str().unwrap()).unwrap();
        assert!(
            ask["says"].as_str().unwrap().contains("g/corpus.toml"),
            "{ask}"
        );
        // Called again without a yes: it waits, and nothing runs.
        let r = call_net(&c, fmt.clone());
        assert!(r.is_error);
        assert_eq!(r.json["ask_id"], id);
        assert!(
            r.json["refused"]
                .as_str()
                .unwrap()
                .contains("waits for their yes"),
            "{}",
            r.json
        );
        assert_eq!(ran(), 1);
        // On the person's word (no page open), the held change goes in.
        let r = call_net(
            &c,
            json!({"command": "model fmt", "args": {"game": "g"}, "confirmed_by_person": true}),
        );
        assert!(!r.is_error, "{}", r.json);
        assert_eq!(r.json["applied"], json!(["g/corpus.toml"]));
        assert_eq!(
            std::fs::read_to_string(top.join("g/corpus.toml")).unwrap(),
            "a\nc\n"
        );
        assert_eq!(ran(), 1, "the command isn't run again");
        // Lines only added run at once.
        let r = call_net(&c, json!({"command": "model fmt", "args": {"game": "add"}}));
        assert!(!r.is_error, "{}", r.json);
        assert_eq!(
            std::fs::read_to_string(top.join("g/corpus.toml")).unwrap(),
            "a\nc\nd\n"
        );
    }

    /// With the page open, the held change is a card, and the person's yes
    /// on it puts the change in; confirmed_by_person isn't enough.
    #[test]
    fn with_the_page_open_a_held_change_goes_in_on_the_card_s_yes() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(dir.path(), "printf 'z\\n' > g/corpus.toml\necho '{}'");
        let mut c = config(dir.path(), &gp);
        let top = games(&mut c, dir.path());
        serve_page(&c);
        let fmt =
            json!({"command": "model fmt", "args": {"game": "g"}, "confirmed_by_person": true});
        let r = call_net(&c, fmt.clone());
        assert!(
            r.is_error,
            "confirmed_by_person doesn't skip showing the change: {}",
            r.json
        );
        let id = r.json["ask_id"].clone();
        let r = call_net(&c, fmt.clone());
        assert!(r.is_error, "and isn't a yes while the page is open");
        answer(&c, &id, true);
        let r = call_net(
            &c,
            json!({"command": "model fmt", "args": {"game": "g"}, "ask_id": id}),
        );
        assert!(!r.is_error, "{}", r.json);
        assert_eq!(
            std::fs::read_to_string(top.join("g/corpus.toml")).unwrap(),
            "z\n"
        );
    }

    /// corpus import guards itself: adding runs at once with no net, and
    /// --replace asks, its card saying what the plan would change.
    #[test]
    fn corpus_import_asks_to_replace_with_its_plan_on_the_card() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(
            dir.path(),
            "case \"$*\" in\n*--write*) printf 'q\\n' > g/corpus.toml; echo '{\"written\": true}' ;;\n*) echo '{\"replaces\": [{\"what\": \"g.a.001\", \"says\": \"changes clip g.a.001: its 40 frames becomes 20\"}]}' ;;\nesac",
        );
        let mut c = config(dir.path(), &gp);
        games(&mut c, dir.path());
        let r = call_net(
            &c,
            json!({"command": "corpus import", "args": {"game": "g", "write": true}}),
        );
        assert!(!r.is_error, "adding runs at once: {}", r.json);
        let r = call_net(
            &c,
            json!({"command": "corpus import", "args": {"game": "g", "write": true, "replace": ["g.a.001"]}}),
        );
        assert!(r.is_error, "{}", r.json);
        assert_eq!(r.json["needs"], "replaces");
        assert_eq!(
            r.json["plan"],
            json!(["changes clip g.a.001: its 40 frames becomes 20"])
        );
        let says = r.json["ask"]["says"].as_str().unwrap();
        assert!(
            says.ends_with("It changes clip g.a.001: its 40 frames becomes 20."),
            "{says}"
        );
    }

    #[test]
    fn with_the_page_open_a_no_keeps_refusing() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(dir.path(), "echo '{\"published\": true}'");
        let c = config(dir.path(), &gp);
        serve_page(&c);
        let publish = json!({"command": "registry publish", "args": {"target": "/tmp/r"}});
        let id = call(&c, publish.clone()).json["ask_id"].clone();
        answer(&c, &id, false);
        for v in [
            json!({"command": "registry publish", "args": {"target": "/tmp/r"}, "ask_id": id}),
            json!({"command": "registry publish", "args": {"target": "/tmp/r"}, "confirmed_by_person": true}),
        ] {
            let r = call(&c, v);
            assert!(r.is_error);
            assert!(
                r.json["refused"].as_str().unwrap().contains("said no"),
                "{}",
                r.json
            );
        }
    }

    #[test]
    fn a_changed_argument_needs_a_new_yes() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(dir.path(), "echo '{\"published\": true}'");
        let c = config(dir.path(), &gp);
        serve_page(&c);
        let id = call(
            &c,
            json!({"command": "registry publish", "args": {"target": "/tmp/r"}}),
        )
        .json["ask_id"]
            .clone();
        answer(&c, &id, true);
        let r = call(
            &c,
            json!({"command": "registry publish", "args": {"target": "/tmp/elsewhere"}, "ask_id": id}),
        );
        assert!(r.is_error);
        assert!(
            r.json["refused"]
                .as_str()
                .unwrap()
                .contains("changed argument")
        );
        assert_ne!(r.json["ask_id"], id);
        // The yes given still holds for the call it was for.
        let r = call(
            &c,
            json!({"command": "registry publish", "args": {"target": "/tmp/r"}}),
        );
        assert!(!r.is_error, "{}", r.json);
    }

    #[test]
    fn a_dry_run_runs_without_a_yes() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(dir.path(), "echo \"{\\\"args\\\": \\\"$*\\\"}\"");
        let c = config(dir.path(), &gp);
        let r = call(
            &c,
            json!({"command": "model gate", "args": {"game": "rooftop", "dry-run": true}}),
        );
        assert!(!r.is_error, "{}", r.json);
        assert_eq!(
            r.json["json"]["args"],
            "--json model gate --dry-run rooftop"
        );
        let r = call(
            &c,
            json!({"command": "model gate", "args": {"game": "rooftop"}}),
        );
        assert!(r.is_error);
        assert_eq!(
            r.json["runs_at_once_with"],
            json!(["dry-run", "score-only"])
        );
    }

    #[test]
    fn servers_and_unknown_commands_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(dir.path(), "exit 0");
        let c = config(dir.path(), &gp);
        let r = call(&c, json!({"command": "serve"}));
        assert!(r.is_error);
        assert!(r.json["refused"].as_str().unwrap().contains("7710"));
        let r = call(&c, json!({"command": "modle gate"}));
        assert!(r.is_error);
        assert_eq!(r.json["nearest"][0], "model gate");
        let r = call(&c, json!({"command": "model"}));
        assert!(r.is_error);
        assert_eq!(r.json["commands"][0]["command"], "model gate");
    }

    #[test]
    fn a_command_past_its_time_limit_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        // It starts a child of its own, which must stop with it. Each
        // writes its process ID first, so the test can look for both after.
        // The limit (5 s) is far beyond what a shell needs to start even on
        // a loaded machine, and far short of the sleeps (120 s): a limit of
        // 1 s once failed when the script hadn't printed yet.
        let pids = dir.path().join("pids");
        let gp = fake_gp(
            dir.path(),
            &format!(
                "echo $$ > '{p}'; sleep 120 & echo $! >> '{p}'; echo started; sleep 120",
                p = pids.display()
            ),
        );
        let c = config(dir.path(), &gp);
        let started = Instant::now();
        let r = call(&c, json!({"command": "env", "timeout_s": 5}));
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "{:?}",
            started.elapsed()
        );
        assert!(r.is_error);
        assert!(
            r.json["timed_out"].as_str().unwrap().contains("after 5 s"),
            "{}",
            r.json
        );
        assert_eq!(r.json["timeout_s"], 5);
        assert_eq!(r.json["stdout"], "started\n");
        assert_eq!(r.json["exit"], Value::Null);
        // The command and its child are gone (a stopped child may linger a
        // moment as a zombie until it is reaped).
        let list = std::fs::read_to_string(&pids).unwrap();
        let ids: Vec<&str> = list.split_whitespace().collect();
        assert_eq!(ids.len(), 2, "{list}");
        let alive = |id: &str| {
            Command::new("ps")
                .args(["-o", "stat=", "-p", id])
                .output()
                .map(|o| {
                    let st = String::from_utf8_lossy(&o.stdout).trim().to_owned();
                    !st.is_empty() && !st.starts_with('Z')
                })
                .unwrap_or(false)
        };
        let t0 = Instant::now();
        while ids.iter().any(|id| alive(id)) {
            assert!(
                t0.elapsed() < Duration::from_secs(10),
                "still running: {list}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn a_command_that_couldnt_run_names_gps_own_error() {
        let dir = tempfile::tempdir().unwrap();
        // As gp prints it: a hint and a bad roms.toml said first, its error last.
        let gp = fake_gp(
            dir.path(),
            "echo 'workflow: port, step 1' >&2; echo 'gp: a roms.toml in /x/games/a reads badly' >&2; echo 'gp: the games in /x/games are under more than one namespace (acme, someone): pass --namespace' >&2; exit 2",
        );
        let c = config(dir.path(), &gp);
        let r = call(&c, json!({"command": "env"}));
        assert!(r.is_error);
        assert_eq!(
            r.json["error"],
            "the games in /x/games are under more than one namespace (acme, someone): pass --namespace"
        );
        let ran = |stderr: &str| Ran {
            code: Some(2),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
            timed_out: false,
            seconds: 0.0,
        };
        // The JSON's own error first; a refusal before it ran; a bare line.
        assert_eq!(
            gp_said(&ran("gp: other"), Some(&json!({"error": "its own"}))).as_deref(),
            Some("its own")
        );
        assert_eq!(
            gp_said(&ran("hint\ngp game new: a worker's shell\n"), None).as_deref(),
            Some("gp game new: a worker's shell")
        );
        assert_eq!(
            gp_said(&ran("hint\nbroke\n"), None).as_deref(),
            Some("broke")
        );
        assert_eq!(gp_said(&ran(""), None), None);
        // A command that ran has no error added.
        let gp = fake_gp(dir.path(), "echo 'gp: fine' >&2; exit 1");
        let r = call(&config(dir.path(), &gp), json!({"command": "env"}));
        assert!(r.json.get("error").is_none(), "{}", r.json);
    }

    #[test]
    fn long_output_is_cut_and_saved_whole() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(
            dir.path(),
            "i=0; while [ $i -lt 4000 ]; do echo \"line $i of the output, padded to be long enough\"; i=$((i+1)); done; echo oops >&2; exit 1",
        );
        let c = config(dir.path(), &gp);
        let r = call(&c, json!({"command": "env"}));
        assert!(!r.is_error, "exit 1 is a check that failed, not an error");
        assert_eq!(r.json["exit"], 1);
        assert_eq!(r.json["stderr"], "oops");
        let cut = &r.json["cut"];
        let total = cut["stdout_bytes"].as_u64().unwrap() as usize;
        assert!(total > MAX_STDOUT);
        assert_eq!(cut["stdout_shown_bytes"], STDOUT_HEAD + STDOUT_TAIL);
        assert_eq!(
            cut["stdout_left_out_bytes"],
            total - STDOUT_HEAD - STDOUT_TAIL
        );
        let saved = PathBuf::from(cut["stdout_saved"].as_str().unwrap());
        let whole = std::fs::read_to_string(&saved).unwrap();
        assert_eq!(whole.len(), total);
        assert!(saved.starts_with(dir.path().join("work").join("gp-run")));
        // The start and the end, where the result is.
        let head = r.json["stdout"].as_str().unwrap();
        let end = r.json["stdout_end"].as_str().unwrap();
        assert_eq!(head.len(), STDOUT_HEAD);
        assert!(whole.starts_with(head) && whole.ends_with(end));
        assert!(end.ends_with("line 3999 of the output, padded to be long enough\n"));
        let note = cut["note"].as_str().unwrap();
        assert!(
            note.starts_with("stdout was cut")
                && note.contains(&format!("{total} bytes"))
                && note.contains("stdout_saved"),
            "{note}"
        );
    }

    #[test]
    fn a_long_json_answer_keeps_its_end_and_its_small_values() {
        let dir = tempfile::tempdir().unwrap();
        // A score's shape: a long list of rows, then the result.
        let gp = fake_gp(
            dir.path(),
            "printf '{\\n  \"rows\": ['; i=0; while [ $i -lt 3000 ]; do printf '{\"clip\": \"clip.%04d\", \"exact\": true},' $i; i=$((i+1)); done; printf '{}],\\n  \"ok\": false,\\n  \"written\": \"/made/up/score.json\",\\n  \"summary\": \"2999 of 3000 clips exact\"\\n}\\n'",
        );
        let c = config(dir.path(), &gp);
        let r = call(
            &c,
            json!({"command": "model gate", "args": {"game": "x", "dry-run": true}}),
        );
        assert!(!r.is_error, "{}", r.json);
        assert!(r.json.get("json").is_none(), "too long to carry whole");
        let cut = &r.json["cut"];
        assert!(
            cut["stdout_bytes"].as_u64().unwrap() as usize > MAX_STDOUT,
            "{cut}"
        );
        let end = r.json["stdout_end"].as_str().unwrap();
        assert!(
            end.contains("\"summary\": \"2999 of 3000 clips exact\""),
            "{end}"
        );
        assert_eq!(
            r.json["json_small"],
            json!({"ok": false, "written": "/made/up/score.json", "summary": "2999 of 3000 clips exact"})
        );
        assert!(cut["json_keys"]["rows"].as_u64().unwrap() > 40_000, "{cut}");
        assert!(cut["note"].as_str().unwrap().contains("json_small"));
        let saved: Value =
            serde_json::from_slice(&std::fs::read(cut["stdout_saved"].as_str().unwrap()).unwrap())
                .unwrap();
        assert_eq!(saved["rows"].as_array().unwrap().len(), 3001);
    }

    #[test]
    fn a_command_that_may_start_laya_waits_for_one() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(dir.path(), "echo '{}'");
        let c = config(dir.path(), &gp);
        let mut t = tree();
        t.commands.push(CommandDef {
            path: "mechanic demo".into(),
            about: String::new(),
            help: String::new(),
            args: vec![],
            json: true,
        });
        let a = || RunArgs {
            command: "mechanic demo".into(),
            args: Map::new(),
            confirmed_by_person: false,
            ask_id: None,
            step: None,
            timeout_s: None,
        };
        let r = run_in(&c, &t, a(), &|_| false).unwrap();
        assert!(r.is_error);
        assert!(r.json["refused"].as_str().unwrap().contains("7720"));
        let r = run_in(&c, &t, a(), &|p| p == 7720).unwrap();
        assert!(!r.is_error, "{}", r.json);
    }

    /// The command runs with the addresses gp_run checked for Laya and gp
    /// serve, so a `--decide` left out asks Laya where gp_run looked.
    #[test]
    fn a_command_is_given_the_addresses_it_was_checked_against() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(
            dir.path(),
            r#"echo "{\"laya\": \"$GP_LAYA_URL\", \"serve\": \"$GP_SERVE_URL\"}""#,
        );
        let c = config(dir.path(), &gp);
        let mut t = tree();
        t.commands.push(CommandDef {
            path: "mechanic demo".into(),
            about: String::new(),
            help: String::new(),
            args: vec![],
            json: true,
        });
        let r = run_in(
            &c,
            &t,
            RunArgs {
                command: "mechanic demo".into(),
                args: Map::new(),
                confirmed_by_person: false,
                ask_id: None,
                step: None,
                timeout_s: None,
            },
            &|p| p == local_port("GP_LAYA_URL", 7720),
        )
        .unwrap();
        assert!(!r.is_error, "{}", r.json);
        let (laya, serve) = (
            local_port("GP_LAYA_URL", 7720),
            local_port("GP_SERVE_URL", 7710),
        );
        assert_eq!(
            r.json["json"],
            json!({"laya": format!("http://127.0.0.1:{laya}"), "serve": format!("http://127.0.0.1:{serve}")})
        );
    }

    #[test]
    fn the_list_is_by_group_with_rules_and_one_command_has_a_schema() {
        let t = tree();
        let all = commands_in(
            &t,
            CommandsArgs {
                path: None,
                find: None,
            },
        )
        .unwrap();
        assert_eq!(all.json["commands"], 5);
        let groups = all.json["groups"].as_array().unwrap();
        assert!(groups.iter().any(|g| g["path"] == "model"));
        assert!(
            all.json["rules"]["ask"]["means"]
                .as_str()
                .unwrap()
                .contains("confirmed_by_person")
        );
        // The developer tools come last.
        assert_eq!(groups.last().unwrap()["path"], "env");
        assert!(groups.last().unwrap()["mark"].is_string());
        let one = commands_in(
            &t,
            CommandsArgs {
                path: Some("gp model gate".into()),
                find: None,
            },
        )
        .unwrap();
        assert_eq!(one.json["rule"], "ask");
        let schema = &one.json["args"];
        assert_eq!(schema["required"], json!(["game"]));
        assert_eq!(schema["properties"]["dry-run"]["type"], "boolean");
        assert_eq!(schema["properties"]["removes"]["type"], "array");
        assert_eq!(
            schema["properties"]["mode"]["enum"],
            json!(["fast", "full"])
        );
        let found = commands_in(
            &t,
            CommandsArgs {
                path: None,
                find: Some("PATCH corpus".into()),
            },
        )
        .unwrap();
        assert_eq!(found.json["found"], 1);
        // A command known to run long shows its own limit; others 300 s.
        assert_eq!(one.json["timeout_s"], 3600);
        let env = commands_in(
            &t,
            CommandsArgs {
                path: Some("env".into()),
                find: None,
            },
        )
        .unwrap();
        assert_eq!(env.json["timeout_s"], 300);
    }

    /// Each command known to run long is a command of the table, gets
    /// longer than the usual limit, and no more than a call may ask; the
    /// ones measured past 300 s are among them.
    #[test]
    fn long_commands_get_a_longer_limit_of_their_own() {
        for (path, s) in policy::LONG {
            assert!(policy::row(path).is_some(), "{path} has no row");
            let d = Duration::from_secs(*s);
            assert!(d > DEFAULT_TIMEOUT && d <= MAX_TIMEOUT, "{path}: {s}");
        }
        for path in [
            "corpus score",
            "model refine",
            "model static",
            "model gate",
            "tool gate",
        ] {
            assert!(default_timeout(path) > DEFAULT_TIMEOUT, "{path}");
        }
        // An arcade game's corpus score took 444 s in a debug build.
        assert!(default_timeout("corpus score") >= Duration::from_secs(900));
        assert_eq!(default_timeout("env"), DEFAULT_TIMEOUT);
    }

    /// A reply stopped at its limit says it timed out, after how long, that
    /// the command may have been fine and slow, and how to ask for longer;
    /// at the most a call may ask, what to do instead.
    #[test]
    fn a_timed_out_reply_says_so_and_how_to_ask_for_longer() {
        let w = timed_out_words("corpus score", Duration::from_secs(1800), None);
        assert!(
            w.starts_with("Timed out: gp corpus score was stopped after 1800 s"),
            "{w}"
        );
        assert!(
            w.contains("known to run long") && w.contains("only slow"),
            "{w}"
        );
        assert!(w.contains("timeout_s set higher, 3600 for example"), "{w}");
        let w = timed_out_words("env", Duration::from_secs(5), Some(5));
        assert!(w.contains("after 5 s, the timeout_s this call gave"), "{w}");
        assert!(w.contains("10 for example"), "{w}");
        let w = timed_out_words("tool gate", MAX_TIMEOUT, Some(9999));
        assert!(
            w.contains("longest gp_run gives") && w.contains("terminal"),
            "{w}"
        );
        assert!(!w.contains("set higher"), "{w}");
    }
}
