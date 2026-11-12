//! The capture event stream (docs/formats/capture-events.md): the events a
//! capture run writes and the inputs people and agents send it, as types,
//! with the reading and appending every party shares. `gp capture` writes
//! the events; `gp serve` reads them for the workspace and takes a
//! person's inputs; `gp mcp` appends an agent's tool calls; `gp host` runs
//! the rounds. Nothing here runs a stage or knows a game.
//!
//! A run is a folder `$GP_HOME/capture/<run>/` holding `events.ndjson`
//! (one [`Event`] a line, written by one writer at a time, which holds
//! `writer.lock`) and `inputs.ndjson` (one [`Input`] a line, appended by
//! anyone in a single write).

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The format's version. A changed meaning bumps it; a new optional field
/// or a new event or input kind doesn't, and readers skip kinds they don't
/// know. Version 0 was the research prototype's.
pub const SCHEMA: u32 = 1;

/// The oldest input version still taken: version 0 inputs mean the same.
pub const OLDEST_INPUT: u32 = 0;

/// The file the JSON Schema is kept in, under `schema/`.
pub const SCHEMA_FILE: &str = "capture-events.json";

/// An error from reading or writing a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub mod requests;

fn err(e: impl std::fmt::Display) -> Error {
    Error(e.to_string())
}

/// Who an event or input is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Actor {
    Pipeline,
    Person,
    Agent,
}

/// How a stage ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Fail,
    Error,
    /// Its inputs matched a saved output, which was replayed.
    Cached,
    /// It had nothing to run on, or its command isn't in this build.
    Skipped,
}

/// A run's state, as the job running its rounds reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    /// A round is running.
    Running,
    /// Inputs arrived; the next round starts once they stop arriving.
    Debounce,
    /// Questions are open; the next round starts when an input arrives.
    Waiting,
    /// A round changed nothing and nothing is open or pending.
    Steady,
    /// The job stopped: its round limit, its idle limit, or a failure.
    Stopped,
}

/// A number and the one before it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Delta {
    pub value: f64,
    pub prev: Option<f64>,
}

/// A candidate answer to a question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Candidate {
    pub value: String,
    pub from: String,
    /// The sources it was read at, as model evidence refs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<Value>,
}

/// What an event says, by kind. In the stream the kind is the event's
/// `kind` and these fields are its `data`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "data")]
pub enum Body {
    /// A run began.
    #[serde(rename = "run.start")]
    RunStart {
        run: String,
        model: String,
        /// The stages each round runs, in order.
        stages: Vec<String>,
    },
    #[serde(rename = "round.start")]
    RoundStart {},
    #[serde(rename = "stage.start")]
    StageStart {
        /// The commands it runs.
        calls: usize,
    },
    #[serde(rename = "stage.end")]
    StageEnd {
        status: Status,
        ms: u64,
        #[serde(default)]
        summary: String,
        /// For a cached stage, the run whose output was replayed, when it
        /// was another run.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_run: Option<String>,
        /// For a cached stage, how the replayed run of it ended.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Status>,
    },
    /// Something a stage found that is worth a line: an asset, a recording,
    /// a fact that differs, a failing check, a gap.
    #[serde(rename = "item")]
    Item {
        /// `asset`, `recording`, `fact`, `check`, `gap`, `label`, `frame`,
        /// `asset_edit` (an agent's sprite edit, docs/sprite-editing.md).
        what: String,
        id: String,
        status: String,
        #[serde(default)]
        text: String,
        /// Pinned as won't model: shown, left out of the counts.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pinned: bool,
        /// For `asset_edit`: its preview files and checks.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
    },
    /// A number that changed. `key` is a scorecard metric name where one
    /// exists (docs/scorecard.md).
    #[serde(rename = "metric")]
    Metric {
        key: String,
        value: f64,
        prev: Option<f64>,
        /// Over this is a failure, for a tracked speed metric.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        budget: Option<f64>,
    },
    /// An open question for the person.
    #[serde(rename = "question")]
    Question {
        id: String,
        text: String,
        #[serde(default)]
        path: String,
        candidates: Vec<Candidate>,
    },
    /// An input applied, refused or noted, echoed with its sender as actor.
    #[serde(rename = "input")]
    Input {
        kind: String,
        /// `applied`, `refused` or `noted`.
        status: String,
        summary: String,
        input: Input,
    },
    /// One row of a loop's table: a refine round, a catalog round.
    #[serde(rename = "row")]
    Row {
        /// `refine` or `catalog`.
        table: String,
        /// The row's number within its table, from 1.
        index: u32,
        cells: BTreeMap<String, Value>,
    },
    #[serde(rename = "round.end")]
    RoundEnd {
        summary: String,
        ms: u64,
        failed: u32,
        /// Every number the round reported.
        metrics: BTreeMap<String, f64>,
        /// Those that changed since the last round.
        deltas: BTreeMap<String, Delta>,
        /// Open questions and gaps, less the pinned ones.
        #[serde(default)]
        open: u32,
    },
    /// What the job running the rounds is doing.
    #[serde(rename = "run.state")]
    RunState {
        state: RunState,
        #[serde(default)]
        reason: String,
    },
    // A play session's kinds (docs/show.md): the original played in the
    // workspace, recorded, marked and explained. Added 2026-10-29; the
    // format stays 1.
    /// A person started playing the original.
    #[serde(rename = "play.start")]
    PlayStart {
        run: String,
        /// The ROM's headerless SHA-256.
        rom: String,
        /// Its registry ID, when it's a registered source.
        #[serde(default)]
        source: Option<String>,
        /// The ROM's name or the source's title.
        #[serde(default)]
        name: String,
        /// The template the model comes from, when there is one.
        #[serde(default)]
        template: Option<String>,
        /// The store the session records into.
        store: String,
        #[serde(default)]
        real_store: bool,
        #[serde(default)]
        notes: String,
    },
    /// A mark on the moments the model gets wrong: frames `from` to `to`.
    #[serde(rename = "play.mark")]
    PlayMark {
        id: u64,
        from: u64,
        to: u64,
        #[serde(default)]
        note: String,
        /// `key` (dropped while playing) or `timeline`.
        by: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        annotation: Option<u64>,
    },
    /// The person stopped playing.
    #[serde(rename = "play.stop")]
    PlayStop {
        frames: u64,
        /// The page's frame hash digest.
        digest: String,
        marks: u64,
    },
    /// The session's input log recorded natively.
    #[serde(rename = "play.recorded")]
    PlayRecorded {
        /// The recording's ID.
        id: String,
        frames: u64,
        #[serde(default)]
        lag_frames: Option<u64>,
        store: String,
        #[serde(default)]
        real_store: bool,
        #[serde(default)]
        digest: Option<String>,
        #[serde(default)]
        recording_digest: Option<String>,
        digest_matches: bool,
        /// `human-play`.
        provenance: String,
    },
    /// A mark written as an annotation of the recording.
    #[serde(rename = "play.annotated")]
    PlayAnnotated {
        mark: u64,
        annotation: u64,
        recording: String,
        #[serde(default)]
        from: u64,
        #[serde(default)]
        to: u64,
    },
    /// Something in a session failed.
    #[serde(rename = "play.error")]
    PlayError { message: String },
    /// `gp show explain` started on the marks.
    #[serde(rename = "show.start")]
    ShowStart {
        marks: Vec<u64>,
        #[serde(default)]
        template: String,
    },
    /// What the original did in a mark's frames that the model didn't.
    #[serde(rename = "show.mark")]
    ShowMark {
        mark: u64,
        from: u64,
        to: u64,
        #[serde(default)]
        note: String,
        summary: String,
        /// One line per thing the original did that the model didn't.
        #[serde(default)]
        missing: Option<Vec<String>>,
        /// The model's side: frames, events, counters, cells, spawns.
        #[serde(default)]
        model: Value,
        /// The original's side: interactions, cells, counters, sound, code.
        #[serde(default)]
        original: Value,
        #[serde(default)]
        recording: String,
    },
    /// The explanation ended.
    #[serde(rename = "show.end")]
    ShowEnd {
        ms: u64,
        marks: u64,
        #[serde(default)]
        has_code: Option<bool>,
        #[serde(default)]
        no_code_why: Option<String>,
        #[serde(default)]
        model_error: Option<String>,
    },
    /// The model's preview for the replay: `building`, `ok`, `skipped` or
    /// `failed`.
    #[serde(rename = "show.preview")]
    ShowPreview {
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        /// The preview's run of the session's input.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        why: Option<String>,
    },
    /// No explanation: the session has no template, so no model.
    #[serde(rename = "show.skipped")]
    ShowSkipped { why: String },
    /// The explanation failed.
    #[serde(rename = "show.error")]
    ShowError { message: String },
    // Point and tell (docs/point-and-tell.md): a person paused the two
    // games, pointed at part of the screen and said what should happen
    // there; a helper or an agent answers. Added 2026-10-29; the format
    // stays 1.
    /// A person's request about a spot of the screen at one frame.
    #[serde(rename = "request")]
    Request {
        /// Its number in the run, from 1 (its input's `item` is
        /// `request:<id>`).
        id: u64,
        /// The frame of the original both games were paused on.
        frame: u64,
        /// The recording or baseline the games were showing, by name.
        #[serde(default)]
        recording: String,
        /// The quick choice: `missing`, `position`, `look`, `sound`,
        /// `react`, or none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        choice: Option<String>,
        /// The person's words.
        #[serde(default)]
        text: String,
        /// What was pointed at, in plain words ("a brick", "the goomba").
        #[serde(default)]
        label: String,
        /// The region in both games and the things resolved there, as
        /// the input's `data` holds them.
        #[serde(default)]
        context: Value,
    },
    /// An answer to a request: what the original does there, what the
    /// model does instead and the model piece that would cover it.
    #[serde(rename = "request.reply")]
    RequestReply {
        /// The request's number.
        request: u64,
        /// This reply's number among the request's replies, from 1.
        reply: u64,
        /// `explained`, `proposed`, `needs_mechanic` or `waiting`.
        status: String,
        /// The explanation in a sentence.
        text: String,
        /// The explanation, a line each.
        #[serde(default)]
        lines: Vec<String>,
        /// The structured explanation: both sides, the model piece, any
        /// proposal.
        #[serde(default)]
        data: Value,
    },
}

/// One event of a run, one a line of `events.ndjson`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    /// [`SCHEMA`].
    pub v: u32,
    /// 1, 2, 3, ... with no gaps.
    pub seq: u64,
    /// Wall time since the run started, for a person to read. Nothing
    /// compares it.
    pub at_ms: u64,
    /// The round it belongs to; 0 for `run.start`.
    pub round: u32,
    pub actor: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(flatten)]
    pub body: Body,
}

/// An input from a person or an agent, one a line of `inputs.ndjson`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub v: u32,
    pub actor: Actor,
    /// `answer` (question, choice), `pin` and `unpin` (path), `note`
    /// (text), `tool` (tool, args, result: an agent's tool call),
    /// `request` (item, text, data: a person's point and tell) or `reply`
    /// (item, text, data: an answer to one).
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// The candidate chosen, from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<u32>,
    /// A model path, question ID or gap ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// For `tool`: the tool's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// For `tool`: its arguments, summarized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    /// For `tool`: its result, summarized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// For `asset_edit`, `accept` and `reject`: the sprite edit,
    /// `<copy>:<seq>` (docs/sprite-editing.md). For `sound_edit`, and for
    /// `accept` and `reject` of one: `sound:<seq>` (docs/sound.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// For `asset_edit`: what the live panel shows of the edit (status,
    /// previews, shots, checks) and where the agent's server keeps the copy.
    /// For `request`: the frame, the region in both games, the things
    /// there, the choice and the recording (docs/point-and-tell.md). For
    /// `reply`: its status, lines and the structured explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Input {
    /// An input of `kind` from `actor`, every other field empty.
    pub fn new(actor: Actor, kind: &str) -> Self {
        Self {
            v: SCHEMA,
            actor,
            kind: kind.into(),
            question: None,
            choice: None,
            path: None,
            text: None,
            tool: None,
            args: None,
            result: None,
            item: None,
            data: None,
        }
    }

    /// Whether the input says enough to be applied.
    pub fn check(&self) -> Result<()> {
        if !(OLDEST_INPUT..=SCHEMA).contains(&self.v) {
            return Err(Error(format!(
                "input version {} isn't {OLDEST_INPUT} to {SCHEMA}",
                self.v
            )));
        }
        if self.actor == Actor::Pipeline {
            return Err(Error("an input is from a person or an agent".into()));
        }
        let fail = |m: &str| Err(Error(m.into()));
        match self.kind.as_str() {
            "answer" if self.question.is_some() && self.choice.is_some_and(|c| c >= 1) => Ok(()),
            "answer" => fail("an answer needs a question and a choice from 1"),
            "pin" | "unpin" if self.path.as_deref().is_some_and(|p| !p.is_empty()) => Ok(()),
            "pin" | "unpin" => fail("a pin needs a path"),
            "note" if self.text.is_some() => Ok(()),
            "note" => fail("a note needs text"),
            "tool" if self.tool.is_some() => Ok(()),
            "tool" => fail("a tool input names its tool"),
            "asset_edit" if self.item.is_some() && self.actor == Actor::Agent => Ok(()),
            "asset_edit" => fail("an asset_edit comes from an agent and names its item"),
            "sound_edit" if self.item.is_some() && self.actor == Actor::Agent => Ok(()),
            "sound_edit" => {
                fail("a sound_edit comes from an agent and names its item (sound:<seq>)")
            }
            "accept" | "reject" if self.item.is_some() => Ok(()),
            "accept" | "reject" => fail("a decision names the edit's item (<copy>:<seq>)"),
            "request"
                if request_number(self.item.as_deref()).is_some()
                    && self.data.as_ref().is_some_and(Value::is_object) =>
            {
                Ok(())
            }
            "request" => fail(
                "a request names its item (request:<n>) and holds its frame and region in data",
            ),
            "reply"
                if self.actor == Actor::Agent
                    && request_number(self.item.as_deref()).is_some()
                    && self.data.as_ref().is_some_and(Value::is_object) =>
            {
                Ok(())
            }
            "reply" => fail(
                "a reply comes from an agent, names the request's item (request:<n>) and holds its status in data",
            ),
            k => Err(Error(format!(
                "unknown input kind {k:?}: answer, pin, unpin, note, tool, asset_edit, sound_edit, accept, reject, request or reply"
            ))),
        }
    }

    /// Whether applying it could change what a round finds. A note or a
    /// tool call only goes into the log; the agent's edits reach the run
    /// as answers and pins.
    pub fn changes_model(&self) -> bool {
        matches!(self.kind.as_str(), "answer" | "pin" | "unpin")
    }
}

/// The number in a request's item, `request:<n>`, from 1.
pub fn request_number(item: Option<&str>) -> Option<u64> {
    item?
        .strip_prefix("request:")?
        .parse::<u64>()
        .ok()
        .filter(|n| *n >= 1)
}

/// Text cut to at most `max` characters, with `...` when cut: a tool's
/// arguments or result as a line of the log.
pub fn summarize(v: &Value, max: usize) -> String {
    let text = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if text.chars().count() <= max {
        return text;
    }
    let mut out: String = text.chars().take(max.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

/// Where runs live under `$GP_HOME`.
pub fn runs_dir(home: &Path) -> PathBuf {
    home.join("capture")
}

/// A run's folder, refusing a name that could reach outside `capture/`.
pub fn run_dir(home: &Path, run: &str) -> Result<PathBuf> {
    if run.is_empty()
        || run.len() > 80
        || run == "cache"
        || !run
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error(format!(
            "a run name is letters, digits, - and _, and not `cache` (got {run:?})"
        )));
    }
    Ok(runs_dir(home).join(run))
}

/// Reads an NDJSON file's whole lines: a torn last line (a write in
/// progress) is left for the next read.
fn whole_lines(file: &Path) -> Result<String> {
    match fs::read_to_string(file) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(Error(format!("reading {}: {e}", file.display()))),
    }
}

/// The events after `after`, at most `limit`, as JSON (a reader keeps a
/// kind it doesn't know).
pub fn events_after(dir: &Path, after: u64, limit: usize) -> Result<Vec<Value>> {
    let text = whole_lines(&dir.join("events.ndjson"))?;
    let mut out = Vec::new();
    for line in text.split_inclusive('\n') {
        if !line.ends_with('\n') {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["seq"].as_u64().is_some_and(|s| s > after) {
            out.push(v);
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

/// A reader that follows an NDJSON file as a writer appends to it, reading
/// only the bytes added since its last read. A torn last line waits for
/// the rest; a line that doesn't parse is skipped.
#[derive(Debug, Default)]
pub struct Tail {
    offset: u64,
    partial: Vec<u8>,
}

impl Tail {
    /// The whole lines added to `file` since the last read, as written
    /// (without their newline). A file not there yet has none.
    pub fn lines(&mut self, file: &Path) -> Result<Vec<String>> {
        use std::io::{Read as _, Seek as _, SeekFrom};
        let mut f = match File::open(file) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error(format!("reading {}: {e}", file.display()))),
        };
        f.seek(SeekFrom::Start(self.offset)).map_err(err)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(err)?;
        self.offset += buf.len() as u64;
        self.partial.extend_from_slice(&buf);
        let mut out = Vec::new();
        while let Some(i) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=i).collect();
            if let Ok(t) = String::from_utf8(line[..line.len() - 1].to_vec()) {
                out.push(t);
            }
        }
        Ok(out)
    }

    /// [`Tail::lines`], each with its parse; a line that doesn't parse is
    /// skipped.
    pub fn read(&mut self, file: &Path) -> Result<Vec<(Value, String)>> {
        Ok(self
            .lines(file)?
            .into_iter()
            .filter_map(|l| Some((serde_json::from_str(&l).ok()?, l)))
            .collect())
    }

    /// Bytes read so far.
    pub fn offset(&self) -> u64 {
        self.offset
    }
}

/// The last event's sequence number, 0 for none.
pub fn last_seq(dir: &Path) -> Result<u64> {
    Ok(events_after(dir, 0, usize::MAX)?
        .last()
        .and_then(|v| v["seq"].as_u64())
        .unwrap_or(0))
}

/// Appends an input to a run's `inputs.ndjson` in one write, so senders in
/// other processes don't interleave within a line. Returns how many inputs
/// the run has.
pub fn append_input(dir: &Path, input: &Input) -> Result<usize> {
    input.check()?;
    if !dir.join("run.json").is_file() {
        return Err(Error(format!("no capture run at {}", dir.display())));
    }
    let mut line = serde_json::to_string(input).map_err(err)?;
    line.push('\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("inputs.ndjson"))
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .map_err(err)?;
    Ok(read_inputs(dir)?.len())
}

/// Every whole input line that parses, in order.
pub fn read_inputs(dir: &Path) -> Result<Vec<Input>> {
    let text = whole_lines(&dir.join("inputs.ndjson"))?;
    Ok(text
        .split_inclusive('\n')
        .filter(|l| l.ends_with('\n'))
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}

/// The run's writer lock: held by the one process writing its events.
/// Dropping it lets the next writer in. The lock is the operating system's
/// (released when the process ends), so a crashed writer leaves none.
pub struct WriterLock {
    _file: File,
    key: PathBuf,
}

/// The runs whose writer lock this process holds now.
static HELD_HERE: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

fn held_here() -> std::sync::MutexGuard<'static, Vec<PathBuf>> {
    HELD_HERE.lock().unwrap_or_else(|e| e.into_inner())
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let mut g = held_here();
        if let Some(i) = g.iter().position(|k| *k == self.key) {
            g.swap_remove(i);
        }
    }
}

/// Takes the run's writer lock, or says who has it. The holder's process
/// id is written into the lock file, so [`held_by_a_leak`] can tell.
pub fn take_writer(dir: &Path) -> Result<WriterLock> {
    let path = dir.join("writer.lock");
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(err)?;
    match f.try_lock() {
        Ok(()) => {
            let key = fs::canonicalize(&path).unwrap_or(path);
            held_here().push(key.clone());
            let _ = f.set_len(0);
            let _ = write!(f, "{}", std::process::id());
            Ok(WriterLock { _file: f, key })
        }
        Err(_) => Err(Error(format!(
            "another process is writing {}; wait for it to finish",
            dir.display()
        ))),
    }
}

/// Whether the lock the run shows as held is only a copy of this process's
/// own, kept by a child that was started while a thread held it and has not
/// reached its `exec` yet. A file descriptor is inherited across a fork, and
/// the lock stays until every copy closes. The lock file names the last
/// process to take it; when that is this one and no thread here holds it,
/// the hold is such a copy and ends within moments.
pub fn held_by_a_leak(dir: &Path) -> bool {
    let path = dir.join("writer.lock");
    let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    if held_here().contains(&key) {
        return false;
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok())
        .is_some_and(|pid| pid == std::process::id())
}

/// Whether a writer holds the run's lock now.
pub fn writer_busy(dir: &Path) -> bool {
    if !dir.join("writer.lock").is_file() {
        return false;
    }
    take_writer(dir).is_err()
}

/// Checks a line of the stream against the format.
pub fn parse_event(v: &Value) -> Result<Event> {
    serde_json::from_value(v.clone()).map_err(|e| Error(format!("{e}: {v}")))
}

/// The JSON Schema of an event line and an input line, as written to
/// `schema/capture-events.json`.
pub fn schema() -> Value {
    let event = schemars::schema_for!(Event);
    let input = schemars::schema_for!(Input);
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Game Printer capture events, format 1",
        "description": "One object a line of a capture run's events.ndjson (event) and inputs.ndjson (input). docs/formats/capture-events.md",
        "$defs": {
            "event": event,
            "input": input,
        },
    })
}

/// The schema as the committed file holds it.
pub fn schema_text() -> String {
    let mut t = serde_json::to_string_pretty(&schema()).expect("a schema serializes");
    t.push('\n');
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_tail_reads_only_the_bytes_added() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("events.ndjson");
        let mut text = String::new();
        for s in 1..=100_000 {
            text.push_str(&format!("{{\"seq\":{s}}}\n"));
        }
        fs::write(&f, &text).unwrap();
        let mut t = Tail::default();
        let first = t.read(&f).unwrap();
        assert_eq!(first.len(), 100_000);
        assert_eq!(t.offset(), text.len() as u64);
        // A torn line waits; the rest of it comes with the next read.
        let mut a = OpenOptions::new().append(true).open(&f).unwrap();
        a.write_all(b"{\"seq\":100001").unwrap();
        assert!(t.read(&f).unwrap().is_empty());
        a.write_all(b"}\n").unwrap();
        let next = t.read(&f).unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].0["seq"], 100_001);
        assert_eq!(next[0].1, "{\"seq\":100001}");
        assert_eq!(t.offset(), (text.len() + "{\"seq\":100001}\n".len()) as u64);
    }

    #[test]
    fn inputs_say_enough() {
        let mut a = Input::new(Actor::Person, "answer");
        assert!(a.check().is_err());
        a.question = Some("derive:levels.main.kill_below".into());
        a.choice = Some(0);
        assert!(a.check().is_err());
        a.choice = Some(2);
        a.check().unwrap();
        let mut p = Input::new(Actor::Person, "pin");
        assert!(p.check().is_err());
        p.path = Some("entities.slug.sound".into());
        p.check().unwrap();
        let mut t = Input::new(Actor::Agent, "tool");
        assert!(t.check().is_err());
        t.tool = Some("model_edit".into());
        t.check().unwrap();
        assert!(!t.changes_model());
        assert!(Input::new(Actor::Pipeline, "note").check().is_err());
        assert!(Input::new(Actor::Person, "delete").check().is_err());
        let mut old = Input::new(Actor::Person, "note");
        old.v = 0;
        old.text = Some("a prototype's input".into());
        old.check().unwrap();
        old.v = 2;
        assert!(old.check().is_err());
    }

    #[test]
    fn run_names_stay_inside() {
        let home = Path::new("/h");
        assert_eq!(
            run_dir(home, "mosshop-1").unwrap(),
            PathBuf::from("/h/capture/mosshop-1")
        );
        for bad in ["", "..", "a/b", "a b", "../x", "cache"] {
            assert!(run_dir(home, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn events_parse_by_kind() {
        let line = json!({"v": 1, "seq": 3, "at_ms": 10, "round": 1, "actor": "pipeline", "stage": "derive", "kind": "stage.end", "data": {"status": "cached", "ms": 0, "summary": "34 of 95", "from_run": "a"}});
        let e = parse_event(&line).unwrap();
        assert!(matches!(
            e.body,
            Body::StageEnd {
                status: Status::Cached,
                ..
            }
        ));
        assert_eq!(serde_json::to_value(&e).unwrap(), line);
        let start = json!({"v": 1, "seq": 1, "at_ms": 0, "round": 1, "actor": "pipeline", "kind": "round.start", "data": {}});
        parse_event(&start).unwrap();
        let bad = json!({"v": 1, "seq": 1, "at_ms": 0, "round": 1, "actor": "pipeline", "kind": "metric", "data": {"key": "x"}});
        assert!(parse_event(&bad).is_err());
    }

    /// A play session's kinds, with the data `gp serve` writes (nulls
    /// where a session has no template, no lag count or no model error).
    #[test]
    fn play_session_kinds_parse() {
        let ev = |kind: &str, data: Value| json!({"v": 1, "seq": 1, "at_ms": 0, "round": 1, "actor": "pipeline", "kind": kind, "data": data});
        let lines = [
            ev(
                "play.start",
                json!({"run": "play-x-1", "rom": "ab", "source": null, "name": "x.nes", "template": null, "store": "/s", "real_store": false, "notes": ""}),
            ),
            ev(
                "play.mark",
                json!({"id": 1, "from": 10, "to": 40, "note": "hit a brick", "by": "key"}),
            ),
            ev(
                "play.stop",
                json!({"frames": 900, "digest": "cd", "marks": 1}),
            ),
            ev(
                "play.recorded",
                json!({"id": "ef", "frames": 900, "lag_frames": null, "store": "/s", "real_store": false, "digest": "cd", "recording_digest": "cd", "digest_matches": true, "provenance": "human-play"}),
            ),
            ev(
                "play.annotated",
                json!({"mark": 1, "annotation": 1, "recording": "ef", "from": 10, "to": 40}),
            ),
            ev("play.error", json!({"message": "no ROM"})),
            ev("show.start", json!({"marks": [1], "template": "mosshop"})),
            ev(
                "show.mark",
                json!({"mark": 1, "from": 10, "to": 40, "note": "", "summary": "s", "missing": null, "model": {"events": []}, "original": null, "recording": "ef"}),
            ),
            ev(
                "show.end",
                json!({"ms": 9, "marks": 1, "has_code": null, "no_code_why": null, "model_error": null}),
            ),
            ev(
                "show.preview",
                json!({"status": "ok", "url": "preview/index.html", "run": "session"}),
            ),
            ev("show.skipped", json!({"why": "no template"})),
            ev("show.error", json!({"message": "failed"})),
        ];
        for l in &lines {
            parse_event(l).unwrap_or_else(|e| panic!("{e}"));
        }
        assert!(parse_event(&ev("play.mark", json!({"id": 1}))).is_err());
        assert!(parse_event(&ev("play.unknown", json!({}))).is_err());
    }

    #[test]
    fn inputs_append_and_read_back_and_a_torn_line_waits() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Input::new(Actor::Person, "note");
        a.text = Some("looks right".into());
        assert!(append_input(dir.path(), &a).is_err(), "no run there");
        fs::write(dir.path().join("run.json"), "{}").unwrap();
        assert_eq!(append_input(dir.path(), &a).unwrap(), 1);
        assert_eq!(append_input(dir.path(), &a).unwrap(), 2);
        assert_eq!(read_inputs(dir.path()).unwrap()[1], a);
        let mut f = OpenOptions::new()
            .append(true)
            .open(dir.path().join("inputs.ndjson"))
            .unwrap();
        f.write_all(b"{\"v\":1,\"actor\"").unwrap();
        assert_eq!(read_inputs(dir.path()).unwrap().len(), 2);
    }

    #[test]
    fn one_writer_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!writer_busy(dir.path()));
        let held = take_writer(dir.path()).unwrap();
        assert!(take_writer(dir.path()).is_err());
        assert!(writer_busy(dir.path()));
        drop(held);
        assert!(!writer_busy(dir.path()));
    }

    #[test]
    fn summaries_are_cut() {
        assert_eq!(summarize(&json!("short"), 10), "short");
        assert_eq!(summarize(&json!({"a": "bcdefghij"}), 10), "{\"a\":\"b...");
    }

    /// schema/ holds what [`schema_text`] gives; write it from gp_capture::schema_text() when the event format changes.
    #[test]
    fn the_schema_file_is_current() {
        let file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema")
            .join(SCHEMA_FILE);
        let committed = fs::read_to_string(&file).unwrap_or_default();
        assert!(
            committed == schema_text(),
            "schema/{SCHEMA_FILE} is stale; run just docs (it is gp_capture::schema_text())"
        );
    }
}
