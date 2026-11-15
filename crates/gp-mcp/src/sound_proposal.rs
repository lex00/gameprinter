//! A sound change the agent proposes for the person's copy, which the
//! person accepts or rejects on their page (issue #4), as a picture for
//! their theme is (gp-cli's theme_picture). `sound_edit` proposes by
//! default: the change is worked out and checked as an edit is, kept here
//! with the clip of the song as it sounds now and with the change, and
//! nothing in the copy changes until the person's ACCEPT. An accepted
//! change can be put back.
//!
//! | File | What |
//! |---|---|
//! | `$GP_HOME/work/proposals/sounds/ID/proposal.json` | One proposal: `{id, copy, copy_dir, title, slot, theme, ops, said, why, clip_song, seconds, set_file, map_file, made_ms, state, words?, answered_ms?, told_ms?, seq?, old_set_sha, old_map_sha, new_set_sha, new_map_sha}`; state is `open`, `accepted`, `rejected`, `undone` or `stale` |
//! | `.../ID/old.wav`, `.../ID/new.wav` | The clip song now and with the change |
//! | `.../ID/old-set.json`, `.../ID/old-map.toml` | The copy's song set and mapping as they were when proposed (absent when the copy had none): the undo |
//! | `.../ID/new-set.json`, `.../ID/new-map.toml` | What ACCEPT writes |
//!
//! A second kind (`kind: "role"`, issue #30) is a new or changed sound for
//! one sound role of the person's own copy of a large-art theme (a theme
//! folder under `$GP_HOME/work/themes/`): a role's file
//! `sound/roles/<role>.wav` or a further take `sound/takes/<role>.<n>.wav`
//! (gp_present::looks::take_file), with its `file` line added to look.txt
//! when the look hasn't got one. gp-cli's `gp theme sound propose DIR
//! --role ROLE --wav FILE` checks it and keeps it here with
//! [`propose_role`]; the card plays the role's sound as it is and the new
//! one, and ACCEPT, REJECT, PUT THE OLD ONE BACK and the agent's reading
//! work as for a song.
//!
//! | File (kind role) | What |
//! |---|---|
//! | `.../ID/proposal.json` | `{id, kind: "role", copy, theme, theme_dir, title, game, role, take, file, why, seconds, old_seconds, made_ms, state, words?, answered_ms?, told_ms?, old_sha, new_sha, old_look_sha, new_look_sha, checked}`; `old_sha` is null when the theme had no file there |
//! | `.../ID/old.wav`, `.../ID/new.wav` | The file as it is (absent when there is none) and the one proposed |
//! | `.../ID/old-look.txt`, `.../ID/new-look.txt` | look.txt as it was and with the file's line |
//!
//! gp serve's `/api/sounds` routes answer from the page; in a terminal
//! `gp theme sound accept`, `reject` and `undo` do the same, and `gp_run`
//! refuses those three: only the person decides (crate::policy). The agent
//! reads the answer with `gp theme sound list`, which marks it told, and
//! hears it come with `next` (wait_s): an open proposal is in
//! `waits_for_yes` as a card (`what: "sound"`), and an answer the agent
//! hasn't read is there too.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::Digest as _;

use crate::sound::{EditArgs, Worked};
use crate::{Config, Reply};

/// How long an answer the agent hasn't read stays in `next`.
const UNREAD_FOR_MS: u64 = 24 * 60 * 60 * 1000;

pub fn dir(home: &Path) -> PathBuf {
    home.join("work").join("proposals").join("sounds")
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn folder(home: &Path, id: &str) -> Option<PathBuf> {
    id_ok(id).then(|| dir(home).join(id))
}

pub fn read(home: &Path, id: &str) -> Option<Value> {
    let t = fs::read_to_string(folder(home, id)?.join("proposal.json")).ok()?;
    serde_json::from_str(&t).ok()
}

fn write_atomic(f: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(d) = f.parent() {
        fs::create_dir_all(d)?;
    }
    let name = f
        .file_name()
        .map_or_else(Default::default, |n| n.to_string_lossy().into_owned());
    let tmp = f.with_file_name(format!(".{name}.partial"));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, f)
}

fn save(home: &Path, v: &Value) -> Result<(), String> {
    let id = v["id"].as_str().unwrap_or_default();
    let d = folder(home, id).ok_or_else(|| format!("{id:?} isn't a proposal's id"))?;
    write_atomic(
        &d.join("proposal.json"),
        serde_json::to_string_pretty(v).expect("JSON").as_bytes(),
    )
    .map_err(|e| format!("writing the proposal {id}: {e}"))
}

/// Every proposal, newest first.
pub fn all(home: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = fs::read_dir(dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| fs::read_to_string(e.path().join("proposal.json")).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .collect();
    out.sort_by_key(|v| std::cmp::Reverse(v["made_ms"].as_u64().unwrap_or(0)));
    out
}

/// The open proposals, oldest first: the queue the person answers.
pub fn queue(home: &Path) -> Vec<Value> {
    let mut q: Vec<Value> = all(home)
        .into_iter()
        .filter(|v| v["state"] == json!("open"))
        .collect();
    q.reverse();
    q
}

/// Answers the agent hasn't read yet, newest first.
pub fn unread(home: &Path) -> Vec<Value> {
    let now = crate::asks::now_ms();
    all(home)
        .into_iter()
        .filter(|v| v["state"] != json!("open") && v["told_ms"].is_null())
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

fn sha(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn sha_of(t: Option<&str>) -> Value {
    t.map_or(Value::Null, |t| json!(sha(t.as_bytes())))
}

/// A proposal's clip: `old` (the song now) or `new` (with the change), as
/// WAV bytes.
pub fn wav(home: &Path, id: &str, which: &str) -> Option<Vec<u8>> {
    if !matches!(which, "old" | "new") {
        return None;
    }
    fs::read(folder(home, id)?.join(format!("{which}.wav"))).ok()
}

fn kept(home: &Path, id: &str, file: &str) -> Option<String> {
    fs::read_to_string(folder(home, id)?.join(file)).ok()
}

// ---------------------------------------------------------------------
// Proposing.

/// Keeps a worked-out edit as a proposal: sound_edit's default.
pub fn propose(config: &Config, a: &EditArgs, w: Worked) -> Result<Reply, String> {
    let home = &config.home;
    let why = a
        .why
        .as_deref()
        .or(a.prompt.as_deref())
        .map(str::trim)
        .unwrap_or("");
    if why.is_empty() {
        return Ok(Reply::refused(json!({
            "ok": false,
            "refused": "give why: one sentence for the person saying why this change. Their page shows it beside the sound now and the new one.",
        })));
    }
    if why.len() > 400 {
        return Ok(Reply::refused(json!({
            "ok": false,
            "refused": "why is longer than one sentence (400 characters at most).",
        })));
    }
    if w.set_after == w.set_before.as_deref().unwrap_or_default()
        && w.map_after == w.map_before.as_deref().unwrap_or_default()
    {
        return Ok(Reply::refused(json!({
            "ok": false,
            "refused": "the ops change nothing in the copy's sound, so there is nothing to propose.",
        })));
    }
    let made = crate::asks::now_ms();
    let new_set_sha = sha(w.set_after.as_bytes());
    let id = format!(
        "{made}-{}",
        &sha(format!("{new_set_sha}{}", w.map_after).as_bytes())[..8]
    );
    let f = dir(home).join(&id);
    let io = |e: std::io::Error| format!("keeping the proposal {id}: {e}");
    fs::create_dir_all(&f).map_err(io)?;
    write_atomic(&f.join("old.wav"), &crate::sound::wav_bytes(&w.before)).map_err(io)?;
    write_atomic(&f.join("new.wav"), &crate::sound::wav_bytes(&w.after)).map_err(io)?;
    if let Some(t) = &w.set_before {
        write_atomic(&f.join("old-set.json"), t.as_bytes()).map_err(io)?;
    }
    if let Some(t) = &w.map_before {
        write_atomic(&f.join("old-map.toml"), t.as_bytes()).map_err(io)?;
    }
    write_atomic(&f.join("new-set.json"), w.set_after.as_bytes()).map_err(io)?;
    write_atomic(&f.join("new-map.toml"), w.map_after.as_bytes()).map_err(io)?;
    let rel = |p: &Path| p.strip_prefix(&w.dir).unwrap_or(p).display().to_string();
    let v = json!({
        "id": id,
        "copy": a.copy,
        "copy_dir": w.dir.display().to_string(),
        "title": w.title,
        "slot": w.slot,
        "theme": a.theme,
        "ops": a.ops,
        "said": w.said,
        "why": why,
        "prompt": a.prompt,
        "clip_song": w.clip_song,
        "seconds": w.seconds,
        "set_file": rel(&w.set_file),
        "map_file": rel(&w.map_file),
        "made_ms": made,
        "state": "open",
        "old_set_sha": sha_of(w.set_before.as_deref()),
        "old_map_sha": sha_of(w.map_before.as_deref()),
        "new_set_sha": new_set_sha,
        "new_map_sha": sha(w.map_after.as_bytes()),
    });
    save(home, &v)?;
    let page = crate::asks::page_served(home);
    let place = queue(home).len();
    let tell = if page {
        "It waits on the person's page as a card that plays the song as it sounds now and with your change, with ACCEPT and REJECT. Nothing in the copy changes until they accept. Tell them in one sentence what you changed and that the card is waiting; then call next with wait_s until waits_for_yes shows it answered, and read their answer with gp theme sound list (gp_run)."
    } else {
        "Their page isn't open: ask them to open it (gp serve), where it waits as a card that plays the sound now and the new one, with ACCEPT and REJECT. Then call next with wait_s and read their answer with gp theme sound list (gp_run)."
    };
    let mut out = v.clone();
    out["ok"] = json!(true);
    out["proposed"] = json!(true);
    out["written"] = json!(false);
    out["page_open"] = json!(page);
    out["waiting"] = json!(place);
    out["before"] = json!(f.join("old.wav"));
    out["after"] = json!(f.join("new.wav"));
    out["next"] = json!(tell);
    Ok(Reply::ok(out))
}

/// A new or changed sound for one role of a large-art theme, checked by
/// the caller (gp-cli's `gp theme sound propose --role`).
pub struct RoleSound<'a> {
    /// The theme's folder (holding package.toml and theme/), under
    /// `$GP_HOME/work/themes/`.
    pub theme: &'a Path,
    /// Its folder holding look.txt.
    pub theme_dir: &'a Path,
    pub title: &'a str,
    pub game: &'a str,
    pub role: &'a str,
    /// 1 for the role's own file, 2 and up for a further take.
    pub take: u32,
    /// The WAV file proposed.
    pub new: &'a [u8],
    /// look.txt with the file's line (the same text when it has it).
    pub new_look: &'a str,
    pub why: &'a str,
    /// Seconds of the new sound and of the one there now (0 for none).
    pub seconds: f64,
    pub old_seconds: f64,
    /// What the caller checked, for the agent.
    pub checked: Value,
}

/// Keeps a role's sound as a proposal: the file there now (if any) and
/// look.txt as they are, and what ACCEPT writes. Nothing in the theme
/// changes.
pub fn propose_role(home: &Path, r: &RoleSound) -> Result<Value, String> {
    if !gp_present::looks::role_name_ok(r.role) || r.take == 0 {
        return Err(format!(
            "`{}` take {} isn't a sound role's file",
            r.role, r.take
        ));
    }
    let file = gp_present::looks::take_file(r.role, r.take);
    let target = r.theme_dir.join(&file);
    let look_file = r.theme_dir.join(gp_present::looks::LOOK_FILE);
    let old = fs::read(&target).ok();
    let old_look = fs::read_to_string(&look_file)
        .map_err(|e| format!("reading {}: {e}", look_file.display()))?;
    if old.as_deref() == Some(r.new) && old_look == r.new_look {
        return Err(format!(
            "the new sound is the same as {file}, byte for byte"
        ));
    }
    let made = crate::asks::now_ms();
    let new_sha = sha(r.new);
    let id = format!("{made}-{}", &new_sha[..8]);
    let f = dir(home).join(&id);
    let io = |e: std::io::Error| format!("keeping the proposal {id}: {e}");
    fs::create_dir_all(&f).map_err(io)?;
    if let Some(o) = &old {
        write_atomic(&f.join("old.wav"), o).map_err(io)?;
    }
    write_atomic(&f.join("new.wav"), r.new).map_err(io)?;
    write_atomic(&f.join("old-look.txt"), old_look.as_bytes()).map_err(io)?;
    write_atomic(&f.join("new-look.txt"), r.new_look.as_bytes()).map_err(io)?;
    let copy = r
        .theme
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let v = json!({
        "id": id,
        "kind": "role",
        "copy": copy,
        "theme": r.theme.display().to_string(),
        "theme_dir": r.theme_dir.display().to_string(),
        "title": r.title,
        "game": r.game,
        "role": r.role,
        "take": r.take,
        "file": file,
        "why": r.why,
        "seconds": (r.seconds * 100.0).round() / 100.0,
        "old_seconds": (r.old_seconds * 100.0).round() / 100.0,
        "made_ms": made,
        "state": "open",
        "old_sha": old.as_deref().map_or(Value::Null, |o| json!(sha(o))),
        "new_sha": new_sha,
        "old_look_sha": sha(old_look.as_bytes()),
        "new_look_sha": sha(r.new_look.as_bytes()),
        "checked": r.checked,
    });
    save(home, &v)?;
    Ok(v)
}

/// What the agent is told after [`propose_role`]: the proposal with its
/// place in the queue and the next step.
pub fn proposed_role(home: &Path, v: &Value) -> Value {
    let page = crate::asks::page_served(home);
    let tell = if page {
        "It waits on the person's page as a card that plays the role's sound as it is and the new one, with ACCEPT and REJECT. Nothing in the theme changes until they accept. Tell them in one sentence what you made and that the card is waiting; then call next with wait_s until waits_for_yes shows it answered, and read their answer with gp theme sound list (gp_run)."
    } else {
        "Their page isn't open: ask them to open it (gp serve), where it waits as a card that plays the sound as it is and the new one, with ACCEPT and REJECT. Then call next with wait_s and read their answer with gp theme sound list (gp_run)."
    };
    let mut out = v.clone();
    out["ok"] = json!(true);
    out["proposed"] = json!(true);
    out["written"] = json!(false);
    out["page_open"] = json!(page);
    out["waiting"] = json!(queue(home).len());
    out["next"] = json!(tell);
    out
}

/// The person's answer on a role's sound (`kind: "role"`).
fn decide_role(
    home: &Path,
    mut v: Value,
    a: Answer,
    words: Option<String>,
) -> Result<Value, String> {
    let id = v["id"].as_str().unwrap_or_default().to_owned();
    let theme_dir = PathBuf::from(v["theme_dir"].as_str().unwrap_or_default());
    let file = v["file"].as_str().unwrap_or_default().to_owned();
    if gp_present::looks::role_take(&file).is_none()
        || !theme_dir.join(gp_present::looks::LOOK_FILE).is_file()
    {
        return Err(format!(
            "the proposal {id} names no theme of yours that is still there"
        ));
    }
    let target = theme_dir.join(&file);
    let look_file = theme_dir.join(gp_present::looks::LOOK_FILE);
    let now_shas = || {
        (
            fs::read(&target)
                .ok()
                .map_or(Value::Null, |b| json!(sha(&b))),
            fs::read(&look_file)
                .ok()
                .map_or(Value::Null, |b| json!(sha(&b))),
        )
    };
    let now = crate::asks::now_ms();
    let state = v["state"].as_str().unwrap_or("").to_owned();
    let io = |p: &Path, e: std::io::Error| format!("writing {}: {e}", p.display());
    match a {
        Answer::Accept | Answer::Reject if state != "open" => {
            return Err(format!("this proposal was answered already ({state})"));
        }
        Answer::Accept => {
            if now_shas() != (v["old_sha"].clone(), v["old_look_sha"].clone()) {
                v["state"] = json!("stale");
                v["answered_ms"] = json!(now);
                v["words"] = json!(
                    "The theme's sound changed after this was proposed, so it wasn't changed."
                );
                save(home, &v)?;
                return Err(format!(
                    "{file} or look.txt changed in the theme after your agent proposed this, so it wasn't changed. Ask your agent to propose it again."
                ));
            }
            let new = wav(home, &id, "new").ok_or("the proposed sound is missing")?;
            let look = kept(home, &id, "new-look.txt").ok_or("the proposed look.txt is missing")?;
            write_atomic(&target, &new).map_err(|e| io(&target, e))?;
            write_atomic(&look_file, look.as_bytes()).map_err(|e| io(&look_file, e))?;
            v["state"] = json!("accepted");
        }
        Answer::Reject => {
            v["state"] = json!("rejected");
        }
        Answer::Undo => {
            if state != "accepted" {
                return Err(format!(
                    "only an accepted sound can be put back (this one is {state})"
                ));
            }
            if now_shas() != (v["new_sha"].clone(), v["new_look_sha"].clone()) {
                return Err(format!(
                    "{file} or look.txt changed in the theme since you accepted this, so the old sound wasn't put back over it."
                ));
            }
            let look = kept(home, &id, "old-look.txt").ok_or("the old look.txt is missing")?;
            match wav(home, &id, "old") {
                Some(old) => write_atomic(&target, &old).map_err(|e| io(&target, e))?,
                // The theme had no file there: it goes, and look.txt's line
                // with it.
                None => fs::remove_file(&target).map_err(|e| io(&target, e))?,
            }
            write_atomic(&look_file, look.as_bytes()).map_err(|e| io(&look_file, e))?;
            v["state"] = json!("undone");
            v["undone_ms"] = json!(now);
        }
    }
    if a != Answer::Undo {
        v["words"] = json!(words);
    }
    v["answered_ms"] = json!(now);
    v["told_ms"] = Value::Null;
    save(home, &v)?;
    Ok(v)
}

// ---------------------------------------------------------------------
// The person's answer.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Accept,
    Reject,
    Undo,
}

/// The person's ACCEPT, REJECT or PUT THE OLD ONE BACK on a proposal.
pub fn decide(home: &Path, id: &str, a: Answer, words: Option<&str>) -> Result<Value, String> {
    let v = read(home, id).ok_or_else(|| format!("there's no sound proposal {id}"))?;
    if v["kind"] == json!("role") {
        let words = words.map(str::trim).filter(|w| !w.is_empty()).map(|w| {
            let mut w = w.to_owned();
            w.truncate(2000);
            w
        });
        return decide_role(home, v, a, words);
    }
    let mut v = v;
    let dir = PathBuf::from(v["copy_dir"].as_str().unwrap_or_default());
    let slot = v["slot"].as_str().unwrap_or_default().to_owned();
    let map_rel = v["map_file"].as_str().unwrap_or_default().to_owned();
    let inside = |p: &str| !p.is_empty() && !p.contains("..") && !Path::new(p).is_absolute();
    if !dir.join(gp_model::MODEL_FILE).is_file()
        || !gp_model::path::is_name(&slot)
        || !inside(&map_rel)
    {
        return Err(format!(
            "the proposal {id} names no copy of yours that is still there"
        ));
    }
    let copy = v["copy"].as_str().unwrap_or_default().to_owned();
    let _lock = gp_model::path::is_name(&copy).then(|| crate::CopyLock::take(&copy));
    let set_file = dir
        .join(crate::sound::SOUND_DIR)
        .join(format!("{slot}.songs.json"));
    let map_file = dir.join(&map_rel);
    let now_shas = || {
        (
            sha_of(fs::read_to_string(&set_file).ok().as_deref()),
            sha_of(fs::read_to_string(&map_file).ok().as_deref()),
        )
    };
    let now = crate::asks::now_ms();
    let words = words.map(str::trim).filter(|w| !w.is_empty()).map(|w| {
        let mut w = w.to_owned();
        w.truncate(2000);
        w
    });
    let state = v["state"].as_str().unwrap_or("").to_owned();
    match a {
        Answer::Accept | Answer::Reject if state != "open" => {
            return Err(format!("this proposal was answered already ({state})"));
        }
        Answer::Accept => {
            if now_shas() != (v["old_set_sha"].clone(), v["old_map_sha"].clone()) {
                v["state"] = json!("stale");
                v["answered_ms"] = json!(now);
                v["words"] = json!(
                    "The copy's sound changed after this was proposed, so it wasn't changed."
                );
                save(home, &v)?;
                return Err(
                    "your copy's sound changed after your agent proposed this, so it wasn't changed. Ask your agent to propose it again."
                        .into(),
                );
            }
            let set = kept(home, id, "new-set.json").ok_or("the proposed songs are missing")?;
            let map = kept(home, id, "new-map.toml").ok_or("the proposed mapping is missing")?;
            let ops: Vec<gp_sound::edit::Op> =
                serde_json::from_value(v["ops"].clone()).map_err(|e| format!("ops: {e}"))?;
            let seq = crate::sound::put_in(
                &dir,
                &slot,
                &ops,
                v["why"].as_str().map(str::to_owned),
                &set,
                &map_file,
                &map,
                v["theme"].as_str(),
            )?;
            v["seq"] = json!(seq);
            v["state"] = json!("accepted");
        }
        Answer::Reject => {
            v["state"] = json!("rejected");
        }
        Answer::Undo => {
            if state != "accepted" {
                return Err(format!(
                    "only an accepted sound can be put back (this one is {state})"
                ));
            }
            if now_shas() != (v["new_set_sha"].clone(), v["new_map_sha"].clone()) {
                return Err(
                    "your copy's sound changed since you accepted this, so the old one wasn't put back over it."
                        .into(),
                );
            }
            crate::sound::take_back(
                &dir,
                &slot,
                kept(home, id, "old-set.json").as_deref(),
                &map_file,
                kept(home, id, "old-map.toml").as_deref(),
                v["seq"].as_u64().map(|s| s as u32),
            )?;
            v["state"] = json!("undone");
            v["undone_ms"] = json!(now);
        }
    }
    if a != Answer::Undo {
        v["words"] = json!(words);
    }
    v["answered_ms"] = json!(now);
    // The agent hears each answer, the undo too.
    v["told_ms"] = Value::Null;
    save(home, &v)?;
    Ok(v)
}

/// How a proposal stands, in words for the agent.
pub fn says(v: &Value) -> String {
    let what = if v["kind"] == json!("role") {
        format!(
            "the sound \"{}\"{} in {}",
            v["role"].as_str().unwrap_or("?"),
            match v["take"].as_u64() {
                Some(t) if t > 1 => format!(" (take {t})"),
                _ => String::new(),
            },
            v["title"]
                .as_str()
                .filter(|t| !t.is_empty())
                .or(v["copy"].as_str())
                .unwrap_or("the theme")
        )
    } else {
        format!(
            "{} in {}",
            v["clip_song"]
                .as_str()
                .map_or_else(|| "the sound".into(), |s| format!("the song \"{s}\"")),
            v["copy"].as_str().unwrap_or("the copy")
        )
    };
    let words = v["words"]
        .as_str()
        .map(|w| format!(" Their words: \"{w}\""))
        .unwrap_or_default();
    match v["state"].as_str().unwrap_or("") {
        "open" => format!("{what}: waiting for the person's answer."),
        "accepted" => format!("{what}: the person accepted it; it is in the copy.{words}"),
        "rejected" => format!("{what}: the person rejected it.{words}"),
        "undone" => format!("{what}: the person accepted it, then put the old sound back."),
        "stale" => format!(
            "{what}: not used, the copy's sound changed after it was proposed. Propose again if it still fits."
        ),
        s => format!("{what}: {s}"),
    }
}

/// The open proposals and the answers, as `gp theme sound list` prints
/// them; reading an answer marks it told. Only `copy`'s when given, and
/// only the last day's answers unless `every`.
pub fn list(home: &Path, copy: Option<&str>, every: bool) -> (Value, String) {
    let now = crate::asks::now_ms();
    let mine = |v: &Value| copy.is_none_or(|c| v["copy"].as_str() == Some(c));
    let open: Vec<Value> = queue(home)
        .into_iter()
        .filter(|v| mine(v))
        .map(|mut v| {
            v["says"] = json!(says(&v));
            v
        })
        .collect();
    let mut answered: Vec<Value> = all(home)
        .into_iter()
        .filter(|v| v["state"] != json!("open") && mine(v))
        .filter(|v| {
            every
                || v["told_ms"].is_null()
                || now.saturating_sub(v["answered_ms"].as_u64().unwrap_or(0)) < UNREAD_FOR_MS
        })
        .collect();
    let mut text: Vec<String> = open
        .iter()
        .map(|v| format!("{} {}", v["id"].as_str().unwrap_or(""), says(v)))
        .collect();
    for v in &mut answered {
        let fresh = v["told_ms"].is_null();
        text.push(format!(
            "{} {}{}",
            v["id"].as_str().unwrap_or(""),
            if fresh { "NEW ANSWER. " } else { "" },
            says(v)
        ));
        v["says"] = json!(says(v));
        if fresh {
            v["told_ms"] = json!(now);
            let _ = save(home, v);
            v["new_answer"] = json!(true);
        }
    }
    if text.is_empty() {
        text.push("No sound proposals.".into());
    }
    (json!({"open": open, "answered": answered}), text.join("\n"))
}
