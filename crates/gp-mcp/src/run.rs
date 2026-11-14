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
//! is JSON) and stderr. Long output is cut, and the whole of it saved.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::policy::{self, Decision, Road};
use crate::{Config, Reply, run_gp};

/// How long a command runs when the call doesn't say.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
/// The longest a call may ask for.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(1800);
/// The most stdout a reply carries; the rest is saved to a file.
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
fn clean_path(p: &str) -> String {
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

fn first_sentence(s: &str) -> String {
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
    /// command: `{"game": "tapper", "dry-run": true, "clip": ["a", "b"]}`.
    /// A switch takes true, a repeated one a list.
    #[serde(default)]
    pub args: Map<String, Value>,
    /// Set this only after the person said yes, in this conversation, to
    /// this exact action (publishing, landing, spending, a big download).
    /// Never set it on your own judgment.
    #[serde(default)]
    pub confirmed_by_person: bool,
    /// Seconds before the command is stopped: default 300, at most 1800.
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
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
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
                stop_group(child.id());
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

/// Stops the process group `pid` leads: the command and what it started.
fn stop_group(pid: u32) {
    #[cfg(unix)]
    for signal in ["-TERM", "-KILL"] {
        let _ = Command::new("kill")
            .arg(signal)
            .arg(format!("-{pid}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        std::thread::sleep(Duration::from_millis(200));
    }
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

/// The output's part of a reply: stdout as JSON or text, stderr, and
/// what was cut and where it is whole.
pub fn output(ran: &Ran, dir: &Path, stem: &str) -> Result<Map<String, Value>, String> {
    let mut o = Map::new();
    let stdout = String::from_utf8_lossy(&ran.stdout);
    let parsed: Option<Value> = serde_json::from_str(stdout.trim()).ok();
    if stdout.len() <= MAX_STDOUT {
        match parsed {
            Some(v) => o.insert("json".into(), v),
            None => o.insert("stdout".into(), json!(stdout)),
        };
    } else {
        let ext = if parsed.is_some() { "json" } else { "txt" };
        let file = save(dir, &format!("{stem}.stdout.{ext}"), &ran.stdout)?;
        o.insert("stdout".into(), json!(cut_at(&stdout, MAX_STDOUT)));
        let mut cut = json!({
            "stdout_shown_bytes": MAX_STDOUT,
            "stdout_bytes": stdout.len(),
            "stdout_saved": file,
            "note": format!(
                "stdout was {} bytes, so only its first {} are here; the whole of it is in the file named in stdout_saved. Narrow the command (its flags) for a shorter answer.",
                stdout.len(),
                MAX_STDOUT
            ),
        });
        if let Some(v) = &parsed {
            cut["json_keys"] = key_sizes(v);
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
        let mut from = stderr.len() - MAX_STDERR;
        while !stderr.is_char_boundary(from) {
            from += 1;
        }
        o.insert("stderr".into(), json!(&stderr[from..]));
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
            json!("stderr was long, so only its end is here; the whole of it is in the file named in stderr_saved"),
        );
    }
    Ok(o)
}

/// The keys under which commands report what they wrote.
const WROTE_KEYS: &[&str] = &[
    "wrote", "written", "writes", "out", "output", "file", "files", "dir", "folder", "path",
    "paths", "into", "session", "copy",
];

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
    match policy::decide(&row, &given, &built.values) {
        Decision::Refused(why) => {
            return Ok(Reply::refused(json!({
                "refused": format!("gp {path} isn't run from a tool: {why}"),
                "rule": row.rule.name(),
            })));
        }
        Decision::NeedsYes(ask) if !a.confirmed_by_person => {
            let mut v = json!({
                "refused": format!(
                    "gp {path} needs the person's yes first: {}. If they say yes to this exact action, call again with confirmed_by_person: true.",
                    ask.before()
                ),
                "rule": row.rule.name(),
                "needs": ask.name(),
            });
            if let policy::Rule::Ask(_, free) = row.rule
                && !free.is_empty()
            {
                v["runs_at_once_with"] = json!(free);
            }
            return Ok(Reply::refused(v));
        }
        _ => {}
    }
    if policy::may_start_laya(&row, &given) && !answers(7720) && !answers(7710) {
        return Ok(Reply::refused(json!({
            "refused": format!(
                "gp {path} asks Laya, the decision server, for moves, and with neither Laya (127.0.0.1:7720) nor gp serve (127.0.0.1:7710) answering it would start a server on port 7720, which a tool never does. Ask the person to start gp serve, then call again."
            ),
            "rule": row.rule.name(),
        })));
    }
    let limit = a
        .timeout_s
        .map_or(DEFAULT_TIMEOUT, Duration::from_secs)
        .min(MAX_TIMEOUT);
    let mut cmd = Command::new(&config.gp);
    if c.json {
        cmd.arg("--json");
    }
    cmd.args(path.split(' ')).args(&built.argv);
    cmd.env("GP_HOME", &config.home);
    match &config.games {
        Some(g) => {
            cmd.env("GP_GAMES_DIR", g).current_dir(g);
        }
        None => {
            cmd.current_dir(&config.work);
        }
    }
    let shown = std::iter::once("gp".to_owned())
        .chain(c.json.then(|| "--json".to_owned()))
        .chain(path.split(' ').map(str::to_owned))
        .chain(built.argv.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let ran = run_limited(cmd, limit)?;
    let stem = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis()),
        path.replace(' ', "-")
    );
    let mut reply = json!({
        "command": shown,
        "rule": row.rule.name(),
        "exit": ran.code,
        "seconds": (ran.seconds * 10.0).round() / 10.0,
    });
    let o = reply.as_object_mut().expect("an object");
    for (k, v) in output(&ran, &saved_dir(config), &stem)? {
        o.insert(k, v);
    }
    let writes = matches!(
        policy::decide(&row, &given, &built.values),
        Decision::Run { writes: true }
    ) || a.confirmed_by_person;
    if writes {
        let wrote: Map<String, Value> = o
            .get("json")
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
    if ran.timed_out {
        o.insert(
            "timed_out".into(),
            json!(format!(
                "stopped after {} s, the time limit, with everything it started; what it printed so far is here. Give it longer with timeout_s (at most {} s), or narrow it with its flags.",
                limit.as_secs(),
                MAX_TIMEOUT.as_secs()
            )),
        );
        return Ok(Reply::refused(reply));
    }
    // Exit 1 is a check that ran and failed, which isn't an error; exit 2
    // (or none) is a command that couldn't run.
    let failed = ran.code != Some(0) && ran.code != Some(1);
    Ok(if failed {
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
                "game": "tapper",
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
                "tapper"
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
        }
    }

    /// A stand-in gp: a shell script that runs `body`.
    fn fake_gp(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let f = dir.join("gp");
        std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir_all(dir.join("work")).unwrap();
        f
    }

    fn call(c: &Config, v: Value) -> Reply {
        let a: RunArgs = serde_json::from_value(v).unwrap();
        run_in(c, &tree(), a, &|_| false).unwrap()
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

    #[test]
    fn a_dry_run_runs_without_a_yes() {
        let dir = tempfile::tempdir().unwrap();
        let gp = fake_gp(dir.path(), "echo \"{\\\"args\\\": \\\"$*\\\"}\"");
        let c = config(dir.path(), &gp);
        let r = call(
            &c,
            json!({"command": "model gate", "args": {"game": "tapper", "dry-run": true}}),
        );
        assert!(!r.is_error, "{}", r.json);
        assert_eq!(r.json["json"]["args"], "--json model gate --dry-run tapper");
        let r = call(
            &c,
            json!({"command": "model gate", "args": {"game": "tapper"}}),
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
        // It starts a child of its own, which must stop with it.
        let gp = fake_gp(dir.path(), "echo started; sleep 30 & sleep 30");
        let c = config(dir.path(), &gp);
        let started = Instant::now();
        let r = call(&c, json!({"command": "env", "timeout_s": 1}));
        assert!(started.elapsed() < Duration::from_secs(15));
        assert!(r.is_error);
        assert!(
            r.json["timed_out"].as_str().unwrap().contains("after 1 s"),
            "{}",
            r.json
        );
        assert_eq!(r.json["stdout"], "started\n");
        assert_eq!(r.json["exit"], Value::Null);
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
        assert_eq!(cut["stdout_shown_bytes"], MAX_STDOUT);
        let total = cut["stdout_bytes"].as_u64().unwrap() as usize;
        assert!(total > MAX_STDOUT);
        let saved = PathBuf::from(cut["stdout_saved"].as_str().unwrap());
        assert_eq!(std::fs::read(&saved).unwrap().len(), total);
        assert!(saved.starts_with(dir.path().join("work").join("gp-run")));
        assert_eq!(r.json["stdout"].as_str().unwrap().len(), MAX_STDOUT);
        assert!(cut["note"].as_str().unwrap().contains("first"));
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
            timeout_s: None,
        };
        let r = run_in(&c, &t, a(), &|_| false).unwrap();
        assert!(r.is_error);
        assert!(r.json["refused"].as_str().unwrap().contains("7720"));
        let r = run_in(&c, &t, a(), &|p| p == 7720).unwrap();
        assert!(!r.is_error, "{}", r.json);
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
    }
}
