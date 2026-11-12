//! Point and tell (docs/point-and-tell.md): a person pauses the two games,
//! points at part of the screen and says what should happen there. That's
//! a `request` input; a helper or an agent answers with a `reply` input.
//! Both are kept in `inputs.ndjson`, which anyone may append to, and each
//! becomes an event of its own kind (`request`, `request.reply`) in the
//! run's stream, so an agent following the run sees it.
//!
//! The event is written by whoever holds the run's writer lock: the run's
//! watcher echoes the input when it applies it, and when no writer holds
//! the lock (a play session, or a run whose watcher stopped) the sender
//! takes it for one line. Either way an event is written once: both look
//! for it first.
//!
//! A request's state is folded from the inputs alone, so it reads the same
//! whether or not an event was written yet.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    Actor, Body, Error, Event, Input, Result, events_after, held_by_a_leak, read_inputs,
    request_number, take_writer,
};

/// The quick choices a request can carry, with the words the workspace
/// shows for each.
pub const CHOICES: &[(&str, &str)] = &[
    ("missing", "this is missing"),
    ("position", "wrong position"),
    ("look", "wrong look"),
    ("sound", "wrong sound"),
    ("react", "should react when I hit it"),
];

/// Words that say what a request is about, for each quick choice: a request
/// sent without a choice gets the choice its text names most (the first
/// in [`CHOICES`] order on a tie), so "make it orange" is about its look.
/// Plain words a person types, matched whole and in lower case; a small
/// fixed list, so the same text always reads the same.
pub const INTENT_WORDS: &[(&str, &[&str])] = &[
    (
        "missing",
        &[
            "missing",
            "gone",
            "absent",
            "disappears",
            "disappeared",
            "vanish",
            "vanishes",
            "nothing",
        ],
    ),
    (
        "position",
        &[
            "position",
            "place",
            "placed",
            "higher",
            "lower",
            "move",
            "moved",
            "further",
            "closer",
            "offset",
            "shifted",
            "misplaced",
            "aligned",
            "align",
        ],
    ),
    (
        "look",
        &[
            "look", "looks", "color", "colour", "colors", "colours", "colored", "coloured",
            "brighter", "darker", "lighter", "bright", "dark", "light", "pale", "shade", "hue",
            "picture", "drawn", "outline", "pixels", "red", "orange", "yellow", "green", "blue",
            "purple", "pink", "brown", "black", "white", "gray", "grey", "gold", "silver", "tan",
        ],
    ),
    (
        "sound",
        &[
            "sound", "sounds", "noise", "hear", "loud", "louder", "quiet", "quieter", "music",
            "beep", "audio", "volume",
        ],
    ),
    (
        "react",
        &[
            "react", "reacts", "break", "breaks", "broken", "hit", "hits", "bump", "bumps",
            "bounce", "shatter", "collect", "touch", "stomp", "open",
        ],
    ),
];

/// The quick choice a request's words name, by [`INTENT_WORDS`]; None
/// when they name none.
pub fn intent(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let mut best: Option<(&'static str, usize)> = None;
    for (choice, list) in INTENT_WORDS {
        let n = words.iter().filter(|w| list.contains(w)).count();
        if n > 0 && best.is_none_or(|(_, b)| n > b) {
            best = Some((choice, n));
        }
    }
    best.map(|(c, _)| c)
}

/// A reply's statuses.
pub const STATUSES: &[&str] = &["explained", "proposed", "needs_mechanic", "waiting"];

/// One reply, as a request's state lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reply {
    pub reply: u64,
    pub actor: Actor,
    pub status: String,
    pub text: String,
    pub data: Value,
}

/// A request with its replies and where it stands.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct State {
    pub id: u64,
    pub item: String,
    pub actor: Actor,
    pub text: String,
    /// The input's data: frame, recording, choice, label, region, things.
    pub data: Value,
    /// `open` until a reply, then the last reply's status; `accepted` or
    /// `rejected` once the person decided on a reply's proposal.
    pub status: String,
    pub replies: Vec<Reply>,
}

/// Every request of the run, oldest first, with its replies.
pub fn list(dir: &Path) -> Result<Vec<State>> {
    Ok(fold(&read_inputs(dir)?))
}

/// One request by number.
pub fn get(dir: &Path, id: u64) -> Result<State> {
    list(dir)?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| Error(format!("no request {id} in {}", dir.display())))
}

/// Folds inputs into requests. Pure, for tests.
pub fn fold(inputs: &[Input]) -> Vec<State> {
    let mut out: Vec<State> = Vec::new();
    for i in inputs {
        let Some(n) = request_number(i.item.as_deref()) else {
            // A decision on a proposal names the proposal's own item.
            if matches!(i.kind.as_str(), "accept" | "reject") {
                for r in &mut out {
                    let proposes = r.replies.iter().any(|x| {
                        x.data["proposal"]["item"].as_str() == i.item.as_deref() && i.item.is_some()
                    });
                    if proposes {
                        r.status = decided(&i.kind);
                    }
                }
            }
            continue;
        };
        match i.kind.as_str() {
            "request" if !out.iter().any(|r| r.id == n) => out.push(State {
                id: n,
                item: format!("request:{n}"),
                actor: i.actor,
                text: i.text.clone().unwrap_or_default(),
                data: i.data.clone().unwrap_or(Value::Null),
                status: "open".into(),
                replies: Vec::new(),
            }),
            "reply" => {
                if let Some(r) = out.iter_mut().find(|r| r.id == n) {
                    let d = i.data.clone().unwrap_or(Value::Null);
                    let status = d["status"].as_str().unwrap_or("explained").to_owned();
                    r.status = status.clone();
                    r.replies.push(Reply {
                        reply: d["reply"].as_u64().unwrap_or(r.replies.len() as u64 + 1),
                        actor: i.actor,
                        status,
                        text: i.text.clone().unwrap_or_default(),
                        data: d,
                    });
                }
            }
            "accept" | "reject" => {
                if let Some(r) = out.iter_mut().find(|r| r.id == n) {
                    r.status = decided(&i.kind);
                }
            }
            _ => {}
        }
    }
    out
}

fn decided(kind: &str) -> String {
    if kind == "accept" {
        "accepted".into()
    } else {
        "rejected".into()
    }
}

/// The number the next request gets.
pub fn next_id(inputs: &[Input]) -> u64 {
    inputs
        .iter()
        .filter(|i| i.kind == "request")
        .filter_map(|i| request_number(i.item.as_deref()))
        .max()
        .unwrap_or(0)
        + 1
}

/// The event an input becomes, for `request` and `reply` inputs.
pub fn event_body(input: &Input) -> Option<Body> {
    let n = request_number(input.item.as_deref())?;
    let d = input.data.clone().unwrap_or(Value::Null);
    let text = input.text.clone().unwrap_or_default();
    match input.kind.as_str() {
        "request" => Some(Body::Request {
            id: n,
            frame: d["frame"].as_u64().unwrap_or(0),
            recording: d["recording"].as_str().unwrap_or_default().to_owned(),
            choice: d["choice"].as_str().map(str::to_owned),
            text,
            label: d["label"].as_str().unwrap_or_default().to_owned(),
            context: d,
        }),
        "reply" => Some(Body::RequestReply {
            request: n,
            reply: d["reply"].as_u64().unwrap_or(1),
            status: d["status"].as_str().unwrap_or("explained").to_owned(),
            text,
            lines: d["lines"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            data: d,
        }),
        _ => None,
    }
}

/// Whether the run's stream already has the event for this body (a
/// request by number, a reply by request and reply number).
pub fn has_event(dir: &Path, body: &Body) -> Result<bool> {
    let want = |v: &Value| match body {
        Body::Request { id, .. } => v["kind"] == "request" && v["data"]["id"] == *id,
        Body::RequestReply { request, reply, .. } => {
            v["kind"] == "request.reply"
                && v["data"]["request"] == *request
                && v["data"]["reply"] == *reply
        }
        _ => false,
    };
    Ok(events_after(dir, 0, usize::MAX)?.iter().any(want))
}

/// What [`post`] did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Posted {
    /// The input as appended, with its item (and a reply's number) filled.
    pub input: Input,
    /// How many inputs the run has.
    pub inputs: usize,
    /// The event's sequence number, when the sender wrote it; `None` when
    /// a writer holds the run and will echo it.
    pub event: Option<u64>,
}

/// Appends a `request` or `reply` input, numbering it, and writes its
/// event when no writer holds the run. The caller serializes posts to one
/// run (gp serve holds a lock), so two requests don't get one number.
pub fn post(dir: &Path, mut input: Input) -> Result<Posted> {
    let inputs = read_inputs(dir)?;
    match input.kind.as_str() {
        "request" => {
            if input.item.is_none() {
                input.item = Some(format!("request:{}", next_id(&inputs)));
            }
        }
        "reply" => {
            let n = request_number(input.item.as_deref())
                .ok_or_else(|| Error("a reply names its request (request:<n>)".into()))?;
            if !fold(&inputs).iter().any(|r| r.id == n) {
                return Err(Error(format!("no request {n} to reply to")));
            }
            let done = inputs
                .iter()
                .filter(|i| i.kind == "reply" && request_number(i.item.as_deref()) == Some(n))
                .count() as u64;
            if let Some(Value::Object(d)) = input.data.as_mut() {
                d.entry("reply").or_insert(json!(done + 1));
                d.entry("status").or_insert(json!("explained"));
            }
        }
        k => return Err(Error(format!("post takes a request or a reply, not {k}"))),
    }
    let n = crate::append_input(dir, &input)?;
    let body = event_body(&input).expect("a numbered request or reply");
    let event = emit_unless_written(dir, input.actor, body)?;
    Ok(Posted {
        input,
        inputs: n,
        event,
    })
}

/// Writes the event when no writer holds the run's lock and the stream
/// doesn't have it yet. Returns its sequence number, or `None`.
pub fn emit_unless_written(dir: &Path, actor: Actor, body: Body) -> Result<Option<u64>> {
    // A hold that is a copy of our own lock in a child still starting up
    // ends within moments; wait it out instead of dropping the event.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let _lock = loop {
        match take_writer(dir) {
            Ok(l) => break l,
            Err(_) if held_by_a_leak(dir) && std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(_) => return Ok(None),
        }
    };
    if has_event(dir, &body)? {
        return Ok(None);
    }
    let run: Value = std::fs::read(dir.join("run.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let started = run["started_ms"].as_u64().unwrap_or(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let seq = crate::last_seq(dir)? + 1;
    let ev = Event {
        v: crate::SCHEMA,
        seq,
        at_ms: now.saturating_sub(started),
        round: run["round"].as_u64().unwrap_or(1).max(1) as u32,
        actor,
        stage: None,
        body,
    };
    let mut line = serde_json::to_string(&ev).map_err(|e| Error(e.to_string()))?;
    line.push('\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("events.ndjson"))
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .map_err(|e| Error(e.to_string()))?;
    Ok(Some(seq))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(text: &str, data: Value) -> Input {
        let mut i = Input::new(Actor::Person, "request");
        i.text = Some(text.into());
        i.data = Some(data);
        i
    }

    fn reply(n: u64, status: &str) -> Input {
        let mut i = Input::new(Actor::Agent, "reply");
        i.item = Some(format!("request:{n}"));
        i.text = Some("the original breaks it".into());
        i.data = Some(json!({"status": status, "lines": ["a", "b"]}));
        i
    }

    #[test]
    fn a_requests_words_name_its_choice() {
        assert_eq!(intent("make it orange"), Some("look"));
        assert_eq!(intent("make them more orange"), Some("look"));
        assert_eq!(intent("Brighter, please!"), Some("look"));
        assert_eq!(intent("it should break when I hit it"), Some("react"));
        assert_eq!(intent("the jump sound is too loud"), Some("sound"));
        assert_eq!(intent("this is missing"), Some("missing"));
        assert_eq!(intent("hello there"), None);
        // Every word list belongs to a quick choice.
        for (c, _) in INTENT_WORDS {
            assert!(CHOICES.iter().any(|(k, _)| k == c), "{c}");
        }
    }

    #[test]
    fn requests_and_replies_need_their_item_and_data() {
        let mut r = request("break it", json!({"frame": 10}));
        assert!(r.check().is_err(), "no item yet");
        r.item = Some("request:0".into());
        assert!(r.check().is_err(), "numbers start at 1");
        r.item = Some("request:1".into());
        r.check().unwrap();
        r.data = Some(json!(3));
        assert!(r.check().is_err(), "data is an object");
        let mut p = reply(1, "explained");
        p.check().unwrap();
        p.actor = Actor::Person;
        assert!(p.check().is_err(), "a reply is an agent's");
    }

    #[test]
    fn post_numbers_requests_writes_events_once_and_folds_replies() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("run.json"), r#"{"started_ms": 0, "round": 2}"#).unwrap();
        let a = post(
            d,
            request(
                "this is missing",
                json!({"frame": 300, "recording": "walk", "choice": "missing", "label": "a brick"}),
            ),
        )
        .unwrap();
        assert_eq!(a.input.item.as_deref(), Some("request:1"));
        assert_eq!(a.event, Some(1));
        let b = post(
            d,
            request("wrong look", json!({"frame": 40, "choice": "look"})),
        )
        .unwrap();
        assert_eq!(b.input.item.as_deref(), Some("request:2"));
        let r = post(d, reply(1, "needs_mechanic")).unwrap();
        assert_eq!(r.input.data.as_ref().unwrap()["reply"], 1);
        assert!(post(d, reply(9, "explained")).is_err(), "no request 9");
        // The events are typed, in order, and each was written once.
        let evs = events_after(d, 0, 100).unwrap();
        assert_eq!(evs.len(), 3);
        let kinds: Vec<&str> = evs.iter().map(|e| e["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["request", "request", "request.reply"]);
        assert_eq!(evs[0]["round"], 2);
        assert_eq!(evs[0]["data"]["label"], "a brick");
        assert_eq!(evs[2]["data"]["lines"], json!(["a", "b"]));
        for e in &evs {
            crate::parse_event(e).unwrap();
        }
        let body = event_body(&a.input).unwrap();
        assert!(has_event(d, &body).unwrap());
        assert_eq!(emit_unless_written(d, Actor::Person, body).unwrap(), None);
        // The fold: request 1 needs a mechanic, 2 is open.
        let all = list(d).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].status, "needs_mechanic");
        assert_eq!(all[0].replies[0].text, "the original breaks it");
        assert_eq!(all[1].status, "open");
        assert_eq!(get(d, 2).unwrap().data["choice"], "look");
    }

    /// A child started while another thread holds the writer lock keeps a
    /// copy of it until its `exec`. That is not a writer, so the event must
    /// not be dropped.
    #[test]
    fn events_survive_children_started_meanwhile() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("run.json"), "{}").unwrap();
        post(d, request("x", json!({"frame": 1}))).unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let spawners: Vec<_> = (0..8)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::SeqCst) {
                        let _ = std::process::Command::new("sh")
                            .args(["-c", "sleep 0.05"])
                            .current_dir("/")
                            .stdin(std::process::Stdio::piped())
                            .status();
                    }
                })
            })
            .collect();
        for _ in 0..300 {
            post(d, reply(1, "explained")).unwrap();
        }
        stop.store(true, Ordering::SeqCst);
        for s in spawners {
            s.join().unwrap();
        }
        assert_eq!(events_after(d, 0, usize::MAX).unwrap().len(), 301);
    }

    #[test]
    fn a_held_writer_echoes_instead() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("run.json"), "{}").unwrap();
        let held = take_writer(d).unwrap();
        let a = post(d, request("x", json!({"frame": 1}))).unwrap();
        assert_eq!(a.event, None);
        assert!(events_after(d, 0, 10).unwrap().is_empty());
        drop(held);
    }

    #[test]
    fn a_decision_on_a_proposal_decides_its_request() {
        let mut r = request("wrong look", json!({"frame": 1}));
        r.item = Some("request:1".into());
        let mut p = reply(1, "proposed");
        p.data = Some(json!({"status": "proposed", "proposal": {"item": "hop:3"}}));
        let mut acc = Input::new(Actor::Person, "accept");
        acc.item = Some("hop:3".into());
        let s = fold(&[r, p, acc]);
        assert_eq!(s[0].status, "accepted");
    }
}
