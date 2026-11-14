//! Tell the agent (game-printer's serve/tell.rs; docs/agents.md, "Telling
//! the agent"): the person pauses a clip, points at the screen and says
//! what is wrong. The note is a work record entry of kind `tell`, `by`
//! "user", with the clip, its frame, the original's frame and the spot or
//! region pointed at, in game pixels, and the clip's score as the
//! background work kept it when it was told (`base`).
//!
//! Delivery: when a clip agent works on the clip, alone or as one of its
//! group, the note is appended to `.gp-inbox/tells.txt` in the agent's
//! worktree, which its prompt tells it to read between steps. Otherwise it
//! waits, and the clip counts as "needs a look" for the coordinator. A
//! reply is a `note` entry with `reply_to` (`gp agent note --reply-to ID`);
//! a tell with a reply is closed.
//!
//! game-printer also credited a tell when the clip got better after it
//! (`resolved` entries and a tally in the page's header); gameprinter's
//! background work doesn't write those yet, so a tell here is open or
//! answered.
//!
//! | Endpoint | What |
//! |---|---|
//! | `POST /api/capture/{run}/clips/{clip}/tell` | `{text, frame?, orig_frame?, at?, region?, echo?}`: adds a tell; with `echo: true` (the page's REFINE queue) it is also the person's turn in the open mechanic's talk |
//! | `GET /api/capture/{run}/tells` | The tells, each with its replies; `?open=1` for those without a reply |

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::game::{Run, run_dir};
use super::work::{self, Entry};
use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};

/// The inbox in a clip agent's worktree.
pub(crate) const INBOX: &str = ".gp-inbox/tells.txt";

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/capture/{run}/clips/{clip}/tell",
            about: "POST {text, frame?, orig_frame?, at?, region?}: tell the agent something about a clip; written to the work record, and to the inbox of the clip agent working on it (gp agent tell)",
            handler: post_tell,
        },
        Route {
            path: "/api/capture/{run}/tells",
            about: "The person's tells with their replies; ?open=1 for those with none, and the clips that need a look (gp agent tells)",
            handler: get_tells,
        },
    ]
}

/// A tell to add.
#[derive(Debug, Clone, Default)]
pub(crate) struct New {
    pub clip: String,
    pub text: String,
    pub frame: Option<u64>,
    pub orig_frame: Option<u64>,
    pub at: Option<[u32; 2]>,
    pub region: Option<[u32; 4]>,
}

/// Where a tell points, in words.
fn where_words(e: &Entry) -> Option<String> {
    if let Some([x, y, w, h]) = e.region {
        return Some(format!("the region {w}x{h} at ({x}, {y})"));
    }
    e.at.map(|[x, y]| format!("the spot ({x}, {y})"))
}

/// "USER TELL on <clip> at frame N: <text>", for a coordinator's log.
pub(crate) fn line(e: &Entry) -> String {
    let clip = e.clip.as_deref().unwrap_or("?");
    match e.frame {
        Some(f) => format!("USER TELL on {clip} at frame {f}: {}", e.text),
        None => format!("USER TELL on {clip}: {}", e.text),
    }
}

fn replies(entries: &[Entry], id: u64) -> Vec<&Entry> {
    entries
        .iter()
        .filter(|e| e.kind == "note" && e.reply_to == Some(id))
        .collect()
}

/// The tells with no reply yet.
pub(crate) fn open(run: &Run) -> Vec<Entry> {
    let es = work::entries(run);
    es.iter()
        .filter(|e| e.kind == "tell" && e.id.is_some_and(|id| replies(&es, id).is_empty()))
        .cloned()
        .collect()
}

/// The clip agent working on `clip`, alone or in its group: its worktree.
fn worktree_on(run: &Run, clip: &str) -> Option<PathBuf> {
    let folder = work::folder(run).join("clip-agents");
    fs::read_dir(folder).ok()?.flatten().find_map(|e| {
        let v: Value = serde_json::from_slice(&fs::read(e.path()).ok()?).ok()?;
        let working = matches!(v["state"].as_str(), Some("starting" | "working"));
        let hit = v["clip"] == clip
            || v["clips"]
                .as_array()
                .is_some_and(|a| a.iter().any(|c| c == clip));
        if working && hit {
            v["worktree"].as_str().map(PathBuf::from)
        } else {
            None
        }
    })
}

fn one_json(es: &[Entry], t: &Entry, run: &Run) -> Value {
    let id = t.id.unwrap_or(0);
    let rs = replies(es, id);
    let clip = t.clip.clone().unwrap_or_default();
    json!({
        "id": id,
        "clip": clip,
        "text": t.text,
        "by": t.by,
        "frame": t.frame,
        "orig_frame": t.orig_frame,
        "at": t.at,
        "region": t.region,
        "base": t.base,
        "when_ms": t.when_ms,
        "open": rs.is_empty(),
        "line": line(t),
        "agent": worktree_on(run, &clip).is_some(),
        "replies": rs.iter().map(|r| json!({"by": r.by, "text": r.text, "when_ms": r.when_ms})).collect::<Vec<_>>(),
    })
}

/// The open tells on these clips as lines for an agent's prompt or brief.
pub(crate) fn brief(run: &Run, clips: &[String]) -> Vec<String> {
    open(run)
        .into_iter()
        .filter(|t| t.clip.as_ref().is_some_and(|c| clips.contains(c)))
        .map(|t| inbox_text(&t))
        .collect()
}

/// A tell as the agent reads it.
fn inbox_text(t: &Entry) -> String {
    let mut s = format!(
        "Tell {} on clip {}",
        t.id.unwrap_or(0),
        t.clip.as_deref().unwrap_or("?")
    );
    if let Some(f) = t.frame {
        s.push_str(&format!(", clip frame {f}"));
    }
    if let Some(f) = t.orig_frame {
        s.push_str(&format!(", original's frame {f}"));
    }
    if let Some(w) = where_words(t) {
        s.push_str(&format!(", pointing at {w} (game pixels)"));
    }
    format!("{s}: {}", t.text)
}

/// Writes the tell into the inbox of the clip agent on its clip; the inbox's
/// path, or None when no agent is on it.
fn deliver(run: &Run, t: &Entry) -> Option<PathBuf> {
    let wt = worktree_on(run, t.clip.as_deref()?)?;
    let file = wt.join(INBOX);
    fs::create_dir_all(file.parent()?).ok()?;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .ok()?;
    writeln!(f, "{}\n", inbox_text(t)).ok()?;
    Some(file)
}

/// Adds a tell: the entry, and the delivery to a clip agent when one is on
/// the clip.
pub(crate) fn add(run: &Run, n: New) -> anyhow::Result<(Entry, Option<PathBuf>)> {
    anyhow::ensure!(!n.text.trim().is_empty(), "say what you want to tell");
    anyhow::ensure!(!n.clip.trim().is_empty(), "name the clip");
    let base = super::background::saved_state(run)["scores"]
        .get(&n.clip)
        .filter(|s| s["error"].is_null())
        .map(|s| {
            json!({"permille": s["permille"], "score": s["score"], "exact": s["exact"], "model": s["model"]})
        });
    let e = work::append(
        run,
        Entry {
            kind: "tell".into(),
            by: "user".into(),
            base,
            clip: Some(n.clip),
            text: n.text.trim().to_owned(),
            frame: n.frame,
            orig_frame: n.orig_frame,
            at: n.at,
            region: n.region,
            ..Entry::default()
        },
    )?;
    let inbox = deliver(run, &e);
    Ok((e, inbox))
}

/// The tells: each, and the clips that need a look (an open tell and no
/// agent on the clip).
pub(crate) fn open_json(run: &Run, only_open: bool) -> Value {
    let es = work::entries(run);
    let list: Vec<Value> = es
        .iter()
        .filter(|e| e.kind == "tell" && e.id.is_some())
        .map(|t| one_json(&es, t, run))
        .filter(|v| !only_open || v["open"] == true)
        .collect();
    let mut look: Vec<String> = list
        .iter()
        .filter(|v| v["open"] == true && v["agent"] == false)
        .filter_map(|v| v["clip"].as_str().map(str::to_owned))
        .collect();
    look.sort();
    look.dedup();
    json!({"run": run.name, "tells": list, "needs_look": look, "fix_ready": work::fix_ready_json(run)})
}

/// `[x, y]` or `[x, y, w, h]` from JSON numbers.
fn spot(v: &Value) -> Option<Vec<u32>> {
    v.as_array()?
        .iter()
        .map(|n| n.as_u64().and_then(|n| u32::try_from(n).ok()))
        .collect()
}

fn post_tell(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let run = run_dir(ctx, params)?;
    let clip = params.get("clip").cloned().unwrap_or_default();
    let v: Value = serde_json::from_slice(&req.body)
        .map_err(|e| Fail::bad_request(format!("the body isn't JSON: {e}")))?;
    let at = match v.get("at").filter(|a| !a.is_null()) {
        None => None,
        Some(a) => match spot(a).as_deref() {
            Some(&[x, y]) => Some([x, y]),
            _ => return Err(Fail::bad_request("`at` is [x, y] in game pixels")),
        },
    };
    let region = match v.get("region").filter(|a| !a.is_null()) {
        None => None,
        Some(a) => match spot(a).as_deref() {
            Some(&[x, y, w, h]) => Some([x, y, w, h]),
            _ => return Err(Fail::bad_request("`region` is [x, y, w, h] in game pixels")),
        },
    };
    let (e, inbox) = add(
        &run,
        New {
            clip,
            text: v["text"].as_str().unwrap_or("").to_owned(),
            frame: v["frame"].as_u64(),
            orig_frame: v["orig_frame"].as_u64(),
            at,
            region,
        },
    )
    .map_err(|e| Fail::bad_request(format!("{e:#}")))?;
    // One inbox: the work record holds the tell, and the open mechanic's
    // talk echoes it, so the conversation column shows what was said.
    let echoed = if v["echo"] == json!(true) {
        super::pages::made(&run.name).and_then(|s| {
            let clip = e.clip.clone().unwrap_or_default();
            s.tell_turn(
                &format!("About clip {clip}: {}", e.text),
                json!({"clip": clip, "id": e.id}),
            )
            .ok()
        })
    } else {
        None
    };
    let es = work::entries(&run);
    Ok(Response::json(&json!({
        "tell": one_json(&es, &e, &run),
        "delivered": inbox.is_some(),
        "echoed": echoed.map(|(n, m)| json!({"mechanic": m, "turn": n})),
        "run": run.name,
    })))
}

fn get_tells(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    let only_open = matches!(req.param("open"), Some("1" | "true" | "yes"));
    Ok(Response::json(&open_json(&run, only_open)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_in(dir: &std::path::Path) -> Run {
        Run {
            name: "g".into(),
            game: dir.join("game"),
            dir: dir.join("serve"),
        }
    }

    /// A tell gets the next number, stays open until a note answers it, and
    /// reads as the agent's inbox line.
    #[test]
    fn a_tell_is_numbered_and_closed_by_a_reply() {
        let d = tempfile::tempdir().unwrap();
        let run = run_in(d.path());
        let (a, inbox) = add(
            &run,
            New {
                clip: "c1".into(),
                text: " the barrel turns early ".into(),
                frame: Some(40),
                at: Some([12, 30]),
                ..New::default()
            },
        )
        .unwrap();
        assert_eq!(a.id, Some(1));
        assert!(inbox.is_none());
        let (b, _) = add(
            &run,
            New {
                clip: "c2".into(),
                text: "late".into(),
                ..New::default()
            },
        )
        .unwrap();
        assert_eq!(b.id, Some(2));
        assert_eq!(open(&run).len(), 2);
        work::append(
            &run,
            Entry {
                kind: "note".into(),
                by: "clip agent".into(),
                reply_to: Some(1),
                text: "looking".into(),
                ..Entry::default()
            },
        )
        .unwrap();
        let v = open_json(&run, true);
        assert_eq!(v["tells"].as_array().unwrap().len(), 1);
        assert_eq!(v["needs_look"], json!(["c2"]));
        assert_eq!(
            inbox_text(&a),
            "Tell 1 on clip c1, clip frame 40, pointing at the spot (12, 30) (game pixels): the barrel turns early"
        );
        assert!(add(&run, New::default()).is_err());
    }
}
