//! A watch session's events: what a run of stage 0 and its search found,
//! one [`WatchEvent`] a line of `$GP_HOME/watch/NAME/events.ndjson`
//! (docs/research/watch-ui.md, section 6). One writer (`gp watch start`)
//! appends them with `seq` 1, 2, 3 and no gaps; `gp watch events`, `gp
//! serve` and `gp mcp` read after a `seq`. Lines hold L2 novelty and
//! summaries, never L3, and no wall time, so the same recipes give the
//! same file.
//!
//! Addresses are named as `gp reach` names them (`C123`, `01:8000`), and
//! `run` is a recipe's hash (`gp reach`'s clip `input`), so a reader can
//! run the same recipe again for detail.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::edges::EdgeKind;

/// The format's version. A breaking change to a kind bumps it.
pub const SCHEMA: u32 = 1;

/// One line of a session's `events.ndjson`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WatchEvent {
    /// [`SCHEMA`].
    pub v: u32,
    /// 1, 2, 3, ... with no gaps.
    pub seq: u64,
    /// The recipe the event came from, by hash, when it came from a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// The frame of that run (frame 1 is the first frame run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<u64>,
    /// The level the event is from (0 to 2), when it is from a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(flatten)]
    pub body: WatchBody,
}

/// A start clip of the session: a recipe from power-on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StartClip {
    pub id: String,
    /// The recipe's hash.
    pub recipe: String,
    pub frames: u64,
}

/// What a session's search aims at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExploreOpts {
    /// `stops`, `unrun`, `unread`, `new-code` or `named`.
    pub aim: String,
    /// The routines aimed at, by the harvest's names.
    pub targets: Vec<String>,
    pub budget: usize,
    pub seed: u64,
}

/// An event's kind (`kind`) and its fields (`data`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "data")]
pub enum WatchBody {
    /// The session began.
    #[serde(rename = "watch.start")]
    Start {
        rom_sha256: String,
        mapper: u16,
        prg_bytes: usize,
        starts: Vec<StartClip>,
        explore: ExploreOpts,
    },
    /// What stage 0 read from the start clips' runs: the session folder's
    /// files a reader fetches, by name with their BLAKE3 hash, and the
    /// counts.
    #[serde(rename = "snapshot")]
    Snapshot {
        files: BTreeMap<String, String>,
        routines: usize,
        read: usize,
        stops: Vec<Stop>,
    },
    /// PRG bytes first executed in the session, as `[start, end)` offset
    /// ranges, with the name of each range's first byte.
    #[serde(rename = "new-code")]
    NewCode {
        ranges: Vec<(u32, u32)>,
        at: Vec<String>,
    },
    /// The first code the session ran in a switched bank.
    #[serde(rename = "bank")]
    Bank { bank: u32, at: String },
    /// A transfer of control first taken in the session.
    #[serde(rename = "edge")]
    Edge {
        edge: EdgeKind,
        from: String,
        to: String,
    },
    /// A value first seen in a pointer a `jmp (ptr)` went through.
    #[serde(rename = "pointer")]
    Pointer { ptr: u16, value: u16, to: String },
    /// The search's progress: candidates run of the budget, clips kept, and
    /// the best rank (distance in calls to a target, then minus the new
    /// instruction starts).
    #[serde(rename = "explore.progress")]
    ExploreProgress {
        ran: usize,
        budget: usize,
        kept: usize,
        best: Option<(u32, i64)>,
    },
    /// The search kept a clip: from a start clip's end, its presses as
    /// spans; `recipe` is the whole recipe from power-on.
    #[serde(rename = "clip.kept")]
    ClipKept {
        id: String,
        start: String,
        presses: Vec<String>,
        frames: usize,
        recipe: String,
        why: String,
        /// The console's state hash at the clip's last frame, as gp-play
        /// computes it (`gp_nes::Nes::state_or_board_free_hash`), so a player
        /// can check its replay.
        end_hash: String,
    },
    /// A run went past a place the static reader stopped: the stop's
    /// transfer, taken to the routine named.
    #[serde(rename = "stop.reached")]
    StopReached { stop: String, routine: String },
    /// Stage 0 read something new after the runs: a routine read.
    #[serde(rename = "fact")]
    Fact { what: String, routine: String },
    /// What the runs did that no rule names yet, from `gp behave`'s
    /// detectors over the start and kept clips: a sound interface, a
    /// background cell kind, a timer or counter, a handler, an element kind,
    /// each in words with the routine or bytes behind it.
    #[serde(rename = "behaviour")]
    Behaviour {
        what: String,
        routine: String,
        why: String,
    },
    /// The session's recipes run again with addresses watched and
    /// instructions hooked (an agent's `watch_watch` or `watch_hook`):
    /// `watch` and `hook` events follow, then `rerun.end`.
    #[serde(rename = "rerun")]
    Rerun {
        watch: Vec<String>,
        hooks: Vec<String>,
        recipes: usize,
    },
    /// A write to a watched address in a rerun: the value, and the writing
    /// instruction named as `gp reach` names it.
    #[serde(rename = "watch")]
    Watch {
        addr: u16,
        value: u8,
        writer: String,
    },
    /// A hooked instruction about to run in a rerun, with its registers.
    #[serde(rename = "hook")]
    Hook {
        at: String,
        a: u8,
        x: u8,
        y: u8,
        s: u8,
        p: u8,
    },
    /// A rerun ended: hits seen and written (at most a cap a recipe, each
    /// address or hook).
    #[serde(rename = "rerun.end")]
    RerunEnd {
        watch_hits: usize,
        hook_hits: usize,
        written: usize,
    },
    /// The session ended.
    #[serde(rename = "watch.end")]
    End {
        ran: usize,
        kept: usize,
        reached: Vec<String>,
        read: usize,
        routines: usize,
    },
}

/// One line of a session's `inputs.ndjson`: what a page or an agent did,
/// appended in one write each by whoever sends it (the events file keeps
/// its one writer). `view` is what the page shows now (recipe, frame,
/// focus), `point` a box pointed at with what is under it, `tool` an
/// agent's call (the tool, its arguments and result in a line each).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatchInput {
    pub v: u32,
    /// `person` or `agent`.
    pub actor: String,
    /// `view`, `point`, `tool` or `note`.
    pub kind: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl WatchInput {
    /// Whether it says enough: a known actor and kind.
    pub fn check(&self) -> Result<(), String> {
        if !matches!(self.actor.as_str(), "person" | "agent") {
            return Err(format!(
                "an input's actor is person or agent, not {:?}",
                self.actor
            ));
        }
        if !matches!(self.kind.as_str(), "view" | "point" | "tool" | "note") {
            return Err(format!(
                "an input's kind is view, point, tool or note, not {:?}",
                self.kind
            ));
        }
        Ok(())
    }
}

/// A place the static reader stopped, from the unread report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stop {
    /// The report's id (`followed, not read@C27D`).
    pub id: String,
    /// The stopping instruction's CPU address.
    pub pc: String,
    /// Where it goes, when known (`8000`).
    pub target: Option<String>,
    /// Routines behind it.
    pub behind: usize,
}

/// The JSON Schema of an event line, as written to
/// `schema/watch-events.json`.
pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Game Printer watch events, format 1",
        "description": "One object a line of a watch session's events.ndjson (event) and inputs.ndjson (input). docs/watch.md",
        "$defs": {
            "event": schemars::schema_for!(WatchEvent),
            "input": schemars::schema_for!(WatchInput),
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

    #[test]
    fn an_event_reads_back_as_written() {
        let e = WatchEvent {
            v: SCHEMA,
            seq: 7,
            run: Some("ab".into()),
            frame: Some(489),
            level: Some(2),
            body: WatchBody::StopReached {
                stop: "followed, not read@C27D".into(),
                routine: "sub_01_8000".into(),
            },
        };
        let line = serde_json::to_string(&e).unwrap();
        assert_eq!(
            line,
            r#"{"v":1,"seq":7,"run":"ab","frame":489,"level":2,"kind":"stop.reached","data":{"stop":"followed, not read@C27D","routine":"sub_01_8000"}}"#
        );
        assert_eq!(serde_json::from_str::<WatchEvent>(&line).unwrap(), e);
        let start = r#"{"v":1,"seq":1,"kind":"explore.progress","data":{"ran":1,"budget":2,"kept":0,"best":null}}"#;
        assert!(serde_json::from_str::<WatchEvent>(start).is_ok());
    }
}
