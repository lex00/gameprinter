//! A sprite edit the agent proposes with `assets_edit` (`place: false`, or
//! an external editor's save through `gp assets open`), answered by the
//! person on their page (issue #4), as a picture for a theme is (gp-cli's
//! theme_picture) and a sound change is (crate::sound_proposal). It works
//! on an NES copy's tiles and on an arcade copy's board sprites alike.
//!
//! The proposal itself stays in the copy's art history
//! (crate::edit_log, `asset-proposals/<seq>.json`), with its pictures in
//! the copy's `previews/edit-<seq>.png` (before and after, close up) and
//! `edit-<seq>-context.png` (the recorded frames, when there are any).
//! Nothing here changes that history's files: an answer is
//! `asset_edit::decide` with `person` as who decided, as `assets_edit`
//! `accept` or `reject` is. What the history has no place for, the
//! person's words and whether the agent has read the answer, is kept
//! beside it here:
//!
//! | File | What |
//! |---|---|
//! | `$GP_HOME/work/proposals/sprites/<copy>-<seq>.json` | One answer the person gave on the page: `{id, copy, seq, state, words?, answered_ms, told_ms?}`; state is `accepted`, `rejected` or `undone` |
//!
//! A proposal is open while its history entry is `proposed`. ACCEPT places
//! it with every check; REJECT drops it; PUT THE OLD ONE BACK, after an
//! ACCEPT, undoes it (a reject of the placed edit, which the history keeps
//! as rejected by the person). The agent reads the answers with
//! `assets_history` (its `answers`, which marks them told) and hears them
//! come with `next`: an open proposal is in `waits_for_yes` as a card
//! (`what: "sprite"`), and an answer the agent hasn't read is there too.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Config;
use crate::edit_log::{self, EditStatus, Entry};

/// What a proposal's reply tells the agent about the card.
pub const ON_THE_PAGE: &str = "It waits on the person's page (gp serve) as a card with the sprite before and after, ACCEPT and REJECT; only they answer it, there or with gp assets proposal in their terminal. assets_history gives their answer and what they wrote (answers), and next lists it when it comes";

/// How long an answer the agent hasn't read stays in `next`.
const UNREAD_FOR_MS: u64 = 24 * 60 * 60 * 1000;

/// The answers' folder.
pub fn dir(home: &Path) -> PathBuf {
    home.join("work").join("proposals").join("sprites")
}

/// Where the page's copies are: `$GP_HOME/work/mcp/NAME/`.
pub fn copies_root(home: &Path) -> PathBuf {
    home.join("work").join("mcp")
}

/// A proposal's id, `<copy>-<seq>`.
pub fn id_of(copy: &str, seq: u64) -> String {
    format!("{copy}-{seq}")
}

/// The copy and edit number an id names.
pub fn parse_id(id: &str) -> Option<(&str, u64)> {
    let (copy, seq) = id.rsplit_once('-')?;
    let seq = seq.parse().ok()?;
    gp_model::path::is_name(copy).then_some((copy, seq))
}

fn answer_file(home: &Path, id: &str) -> Option<PathBuf> {
    parse_id(id).map(|(c, s)| dir(home).join(format!("{}.json", id_of(c, s))))
}

fn read_answer(home: &Path, id: &str) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(answer_file(home, id)?).ok()?).ok()
}

fn save_answer(home: &Path, v: &Value) -> Result<(), String> {
    let id = v["id"].as_str().unwrap_or_default();
    let f = answer_file(home, id).ok_or_else(|| format!("{id:?} isn't a proposal's id"))?;
    fs::create_dir_all(dir(home)).map_err(|e| format!("making {}: {e}", dir(home).display()))?;
    let tmp = f.with_extension("json.partial");
    fs::write(&tmp, serde_json::to_string_pretty(v).expect("JSON"))
        .and_then(|()| fs::rename(&tmp, &f))
        .map_err(|e| format!("writing the answer to {id}: {e}"))
}

fn copy_dir(home: &Path, copy: &str) -> Option<PathBuf> {
    let d = copies_root(home).join(copy);
    (gp_model::path::is_name(copy) && d.join(gp_model::MODEL_FILE).is_file()).then_some(d)
}

/// When a proposal was made: its file's time.
fn made_ms(copy_dir: &Path, seq: u64) -> u64 {
    fs::metadata(
        copy_dir
            .join(edit_log::PROPOSALS_DIR)
            .join(format!("{seq}.json")),
    )
    .and_then(|m| m.modified())
    .ok()
    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
    .map_or(0, |d| d.as_millis() as u64)
}

fn preview_file(copy_dir: &Path, seq: u64, which: &str) -> Option<PathBuf> {
    let name = match which {
        "edit" => format!("edit-{seq}.png"),
        "context" => format!("edit-{seq}-context.png"),
        _ => return None,
    };
    Some(copy_dir.join(crate::asset_edit::PREVIEW_DIR).join(name))
}

/// A proposal as the page and `next` show it.
fn shown(copy: &str, d: &Path, e: &Entry, state: &str) -> Value {
    let has = |w: &str| preview_file(d, e.seq, w).is_some_and(|f| f.is_file());
    json!({
        "id": id_of(copy, e.seq),
        "copy": copy,
        "seq": e.seq,
        "subject": e.subject(),
        "slot": e.slot,
        "board": crate::board_trip::is_board_entry(e),
        "ops": e.op_names(),
        "frames": e.frames,
        "editor": e.editor,
        "why": e.prompt,
        "state": state,
        "made_ms": made_ms(d, e.seq),
        "preview": has("edit"),
        "context": has("context"),
    })
}

fn copies(home: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = fs::read_dir(copies_root(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            copy_dir(home, &name).map(|d| (name, d))
        })
        .collect();
    out.sort();
    out
}

/// The open proposals of every copy, oldest first: the queue the person
/// answers.
pub fn queue(home: &Path) -> Vec<Value> {
    let mut q: Vec<Value> = Vec::new();
    for (copy, d) in copies(home) {
        for e in edit_log::proposals(&d).unwrap_or_default() {
            if e.status == EditStatus::Proposed {
                q.push(shown(&copy, &d, &e, "open"));
            }
        }
    }
    q.sort_by_key(|v| (v["made_ms"].as_u64().unwrap_or(0), v["seq"].as_u64()));
    q
}

/// The person's answers given on the page, newest first, each with the
/// proposal as the history has it.
pub fn answered(home: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = fs::read_dir(dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .filter_map(|a| {
            let copy = a["copy"].as_str()?;
            let seq = a["seq"].as_u64()?;
            let d = copy_dir(home, copy)?;
            let e = edit_log::find(&d, seq).ok()??;
            let mut v = shown(copy, &d, &e, a["state"].as_str().unwrap_or(""));
            for k in ["words", "answered_ms", "told_ms"] {
                v[k] = a[k].clone();
            }
            Some(v)
        })
        .collect();
    out.sort_by_key(|v| std::cmp::Reverse(v["answered_ms"].as_u64().unwrap_or(0)));
    out
}

/// Answers the agent hasn't read yet, newest first.
pub fn unread(home: &Path) -> Vec<Value> {
    let now = crate::asks::now_ms();
    answered(home)
        .into_iter()
        .filter(|v| v["told_ms"].is_null())
        .filter(|v| now.saturating_sub(v["answered_ms"].as_u64().unwrap_or(0)) < UNREAD_FOR_MS)
        .collect()
}

/// What `next` lists: the queue (each with its place in it) and the
/// answers the agent hasn't read.
pub fn for_next(home: &Path) -> Vec<Value> {
    let q = queue(home);
    let n = q.len();
    let mut out: Vec<Value> = q
        .into_iter()
        .enumerate()
        .map(|(i, mut v)| {
            v["place"] = json!(i + 1);
            v["of"] = json!(n);
            v
        })
        .collect();
    out.extend(unread(home));
    out
}

/// A proposal's picture: `edit` (before and after, close up) or `context`
/// (the recorded frames), as PNG bytes.
pub fn png(home: &Path, id: &str, which: &str) -> Option<Vec<u8>> {
    let (copy, seq) = parse_id(id)?;
    fs::read(preview_file(&copy_dir(home, copy)?, seq, which)?).ok()
}

/// The person's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Accept,
    Reject,
    Undo,
}

/// The person's ACCEPT, REJECT or PUT THE OLD ONE BACK on a proposal:
/// the proposal as shown after it, or why it can't be done.
pub fn decide(config: &Config, id: &str, a: Answer, words: Option<&str>) -> Result<Value, String> {
    let home = &config.home;
    let (copy, seq) = parse_id(id).ok_or_else(|| format!("{id:?} isn't a sprite proposal's id"))?;
    let copy = copy.to_owned();
    let d = config.existing_copy(&copy)?;
    let _lock = crate::CopyLock::take(&copy);
    let words = words.map(str::trim).filter(|w| !w.is_empty()).map(|w| {
        let mut w = w.to_owned();
        w.truncate(2000);
        w
    });
    let e = edit_log::find(&d, seq)?.ok_or_else(|| format!("there's no edit {seq} in {copy}"))?;
    let before = read_answer(home, id);
    let state = match a {
        Answer::Accept | Answer::Reject => {
            if e.status != EditStatus::Proposed {
                return Err(format!(
                    "edit {seq} of {copy} isn't waiting for an answer: it is {}",
                    e.status.name()
                ));
            }
            let accept = a == Answer::Accept;
            let r = crate::asset_edit::decide(config, &copy, seq, accept, "person")?;
            if r.is_error || r.json["ok"] == json!(false) {
                return Err(r.json["error"]
                    .as_str()
                    .or(r.json["refused"].as_str())
                    .unwrap_or("the edit couldn't be placed")
                    .to_owned());
            }
            if accept { "accepted" } else { "rejected" }
        }
        Answer::Undo => {
            let ok = before
                .as_ref()
                .is_some_and(|b| b["state"] == json!("accepted"))
                && e.status == EditStatus::Accepted;
            if !ok {
                return Err(format!(
                    "edit {seq} of {copy} isn't one you accepted that is still in place"
                ));
            }
            let r = crate::asset_edit::decide(config, &copy, seq, false, "person")?;
            if r.is_error || r.json["ok"] == json!(false) {
                return Err(r.json["error"]
                    .as_str()
                    .or(r.json["refused"].as_str())
                    .unwrap_or("the edit couldn't be undone")
                    .to_owned());
            }
            "undone"
        }
    };
    let v = json!({
        "id": id_of(&copy, seq),
        "copy": copy,
        "seq": seq,
        "state": state,
        "words": words.or_else(|| before.and_then(|b| b["words"].as_str().map(str::to_owned))),
        "answered_ms": crate::asks::now_ms(),
        "told_ms": null,
    });
    save_answer(home, &v)?;
    let e = edit_log::find(&d, seq)?.unwrap_or(e);
    let mut out = shown(&copy, &d, &e, state);
    for k in ["words", "answered_ms", "told_ms"] {
        out[k] = v[k].clone();
    }
    Ok(out)
}

/// Why `assets_edit` won't accept or reject a proposal, or an edit the
/// person decided: only the person does (crate::policy).
pub const PERSON_DECIDES: &str = "only the person decides a proposed sprite edit, with ACCEPT, REJECT or PUT THE OLD ONE BACK on their page (or gp assets proposal accept, reject or undo in their terminal). Propose with assets_edit (place: false), wait with next (wait_s) until waits_for_yes shows it answered, and read their answer with assets_history (answers).";

/// Whether the agent may accept or reject edit `seq` of a copy itself:
/// not while it is an open proposal, nor once the person decided it.
pub fn agent_may_decide(copy_dir: &Path, seq: u64) -> Result<bool, String> {
    Ok(match edit_log::find(copy_dir, seq)? {
        Some(e) => e.status != EditStatus::Proposed && e.decided_by.as_deref() != Some("person"),
        None => true,
    })
}

/// `gp assets proposal list`: the open proposals, then the person's
/// answers newest first, each marked read.
pub fn list(home: &Path, copy: Option<&str>) -> (Value, String) {
    let keep = |v: &Value| copy.is_none_or(|c| v["copy"] == json!(c));
    let open: Vec<Value> = queue(home).into_iter().filter(keep).collect();
    let answered: Vec<Value> = answered(home).into_iter().filter(keep).collect();
    let now = crate::asks::now_ms();
    for v in &answered {
        if let Some(mut a) = read_answer(home, v["id"].as_str().unwrap_or_default())
            && a["told_ms"].is_null()
        {
            a["told_ms"] = json!(now);
            let _ = save_answer(home, &a);
        }
    }
    let mut text = String::new();
    if open.is_empty() {
        text.push_str("No sprite edit waits for an answer.\n");
    }
    for v in &open {
        text.push_str(&format!(
            "{} waits: the {} in {}. {}\n",
            v["id"].as_str().unwrap_or(""),
            v["subject"].as_str().unwrap_or("sprite"),
            v["copy"].as_str().unwrap_or(""),
            v["why"].as_str().unwrap_or("")
        ));
    }
    for v in &answered {
        text.push_str(&format!(
            "{} {}: the {} in {}.{}\n",
            v["id"].as_str().unwrap_or(""),
            v["state"].as_str().unwrap_or(""),
            v["subject"].as_str().unwrap_or("sprite"),
            v["copy"].as_str().unwrap_or(""),
            v["words"]
                .as_str()
                .map_or_else(String::new, |w| format!(" The person said: \"{w}\""))
        ));
    }
    (json!({ "open": open, "answered": answered }), text)
}

/// The person's answers on a copy's proposals, for `assets_history`, each
/// marked read by the agent.
pub fn tell(home: &Path, copy: &str) -> Vec<Value> {
    let now = crate::asks::now_ms();
    let mut out = Vec::new();
    for v in answered(home)
        .into_iter()
        .filter(|v| v["copy"] == json!(copy))
    {
        let id = v["id"].as_str().unwrap_or_default().to_owned();
        if let Some(mut a) = read_answer(home, &id)
            && a["told_ms"].is_null()
        {
            a["told_ms"] = json!(now);
            let _ = save_answer(home, &a);
        }
        out.push(json!({
            "seq": v["seq"],
            "state": v["state"],
            "words": v["words"],
            "answered_ms": v["answered_ms"],
            "new": v["told_ms"].is_null(),
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_names_a_copy_and_an_edit() {
        assert_eq!(parse_id("moss_hop-12"), Some(("moss_hop", 12)));
        assert_eq!(id_of("moss_hop", 12), "moss_hop-12");
        assert_eq!(parse_id("../x-1"), None);
        assert_eq!(parse_id("copy-x"), None);
        assert_eq!(parse_id("copy"), None);
    }
}
