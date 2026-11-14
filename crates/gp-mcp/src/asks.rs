//! The person's yes, as a record a click made. When `gp_run` refuses a
//! command for want of the person's yes, it writes an ask here; the page
//! (gp serve's `/api/asks`) shows each open ask as a card with YES and NO
//! and writes the answer; a later `gp_run` of the same command with the
//! same arguments runs only on a recorded yes not yet used, and uses it.
//! The tool `next` lists the open asks in `waits_for_yes`.
//!
//! | File | What |
//! |---|---|
//! | `$GP_HOME/work/asks/ID.json` | One ask: `{id, command, argv, key, game, needs, says, cost?, made_ms, state, answered_ms?, used_ms?}`; state is `open`, `yes`, `no` or `used` |
//! | `$GP_HOME/work/serve/page.json` | `{pid, port}`: the gp serve that answers the cards for this home, written when it starts |
//!
//! The yes guards against a careless agent, not a hostile one: a program
//! that can write the home can write a yes (PRODUCT-REVIEW.md, "the yes").

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::Digest as _;

use crate::policy::Ask;

/// How long an open ask waits for an answer before it no longer shows.
pub const OPEN_FOR_MS: u64 = 60 * 60 * 1000;
/// How long a yes holds before it is used.
pub const YES_FOR_MS: u64 = 30 * 60 * 1000;
/// How long a no stops the same command being asked again.
pub const NO_FOR_MS: u64 = 10 * 60 * 1000;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn dir(home: &Path) -> PathBuf {
    home.join("work").join("asks")
}

/// The record of the gp serve that answers this home's cards.
pub fn page_file(home: &Path) -> PathBuf {
    home.join("work").join("serve").join("page.json")
}

/// Whether a process is running.
pub fn alive(pid: u64) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Whether a gp serve is running for this home, so the person has a page
/// to answer on.
pub fn page_served(home: &Path) -> bool {
    std::fs::read_to_string(page_file(home))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["pid"].as_u64())
        .is_some_and(alive)
}

/// Written by gp serve when it starts.
pub fn mark_page_served(home: &Path, port: u16) -> std::io::Result<()> {
    let f = page_file(home);
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(
        f,
        json!({"pid": std::process::id(), "port": port}).to_string(),
    )
}

/// The key of one exact call: the command and its arguments as gp gets
/// them.
pub fn key(command: &str, argv: &[String]) -> String {
    let mut h = sha2::Sha256::new();
    h.update(command.as_bytes());
    for a in argv {
        h.update([0u8]);
        h.update(a.as_bytes());
    }
    h.finalize()
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A game's folder name: letters, digits, `-` and `_`.
pub fn game_ok(g: &str) -> bool {
    !g.is_empty()
        && g.len() <= 64
        && g.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn file(home: &Path, id: &str) -> Option<PathBuf> {
    id_ok(id).then(|| dir(home).join(format!("{id}.json")))
}

/// One ask by its id.
pub fn read(home: &Path, id: &str) -> Option<Value> {
    let t = std::fs::read_to_string(file(home, id)?).ok()?;
    serde_json::from_str(&t).ok()
}

fn write(home: &Path, v: &Value) -> Result<(), String> {
    let id = v["id"].as_str().unwrap_or_default();
    let f = file(home, id).ok_or_else(|| format!("{id:?} isn't an ask's id"))?;
    std::fs::create_dir_all(dir(home)).map_err(|e| format!("making the asks' folder: {e}"))?;
    let tmp = f.with_extension("partial");
    std::fs::write(&tmp, serde_json::to_string_pretty(v).expect("JSON"))
        .and_then(|()| std::fs::rename(&tmp, &f))
        .map_err(|e| format!("writing {}: {e}", f.display()))
}

/// Every ask, newest first.
pub fn all(home: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = std::fs::read_dir(dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .collect();
    out.sort_by_key(|v| std::cmp::Reverse(v["made_ms"].as_u64().unwrap_or(0)));
    out
}

/// Whether an ask still waits for the person.
pub fn is_open(v: &Value, now: u64) -> bool {
    v["state"] == json!("open")
        && now.saturating_sub(v["made_ms"].as_u64().unwrap_or(0)) < OPEN_FOR_MS
}

/// Whether an ask holds a yes not yet used and not too old.
pub fn is_yes(v: &Value, now: u64) -> bool {
    v["state"] == json!("yes")
        && now.saturating_sub(v["answered_ms"].as_u64().unwrap_or(0)) < YES_FOR_MS
}

/// The asks a person may still answer, and the yeses not yet used: what
/// the page shows and `next` lists.
pub fn pending(home: &Path) -> Vec<Value> {
    let now = now_ms();
    all(home)
        .into_iter()
        .filter(|v| is_open(v, now) || is_yes(v, now))
        .collect()
}

/// What the person answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
}

/// The page's YES or NO on an open ask.
pub fn answer(home: &Path, id: &str, a: Answer) -> Result<Value, String> {
    let mut v = read(home, id).ok_or_else(|| format!("there's no ask {id}"))?;
    if !is_open(&v, now_ms()) {
        return Err(format!(
            "this ask was answered already, or is too old ({})",
            v["state"].as_str().unwrap_or("?")
        ));
    }
    v["state"] = json!(match a {
        Answer::Yes => "yes",
        Answer::No => "no",
    });
    v["answered_ms"] = json!(now_ms());
    write(home, &v)?;
    Ok(v)
}

/// What the runner finds for a call that needs a yes.
#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    /// A yes for this exact call, now used: run it.
    Yes(Value),
    /// The person said no to it lately.
    No(Value),
    /// It waits for the person's answer.
    Open(Value),
    /// The ask named is for another command or other arguments.
    Other(Value),
    /// Nothing yet.
    None,
}

/// Looks for the person's answer to a call with this key, by `id` when the
/// agent names an ask; a yes found is marked used, so it runs once.
pub fn find(home: &Path, id: Option<&str>, key: &str) -> Found {
    let now = now_ms();
    let candidates: Vec<Value> = match id {
        Some(id) => match read(home, id) {
            Some(v) if v["key"] != json!(key) => return Found::Other(v),
            Some(v) => vec![v],
            None => Vec::new(),
        },
        None => all(home)
            .into_iter()
            .filter(|v| v["key"] == json!(key))
            .collect(),
    };
    if let Some(mut v) = candidates.iter().find(|v| is_yes(v, now)).cloned() {
        v["state"] = json!("used");
        v["used_ms"] = json!(now);
        if write(home, &v).is_ok() {
            return Found::Yes(v);
        }
    }
    if let Some(v) = candidates.iter().find(|v| is_open(v, now)) {
        return Found::Open(v.clone());
    }
    if let Some(v) = candidates.iter().find(|v| {
        v["state"] == json!("no")
            && (id.is_some()
                || now.saturating_sub(v["answered_ms"].as_u64().unwrap_or(0)) < NO_FOR_MS)
    }) {
        return Found::No(v.clone());
    }
    Found::None
}

/// A new open ask.
pub struct New<'a> {
    pub command: &'a str,
    pub argv: &'a [String],
    pub values: &'a [(String, String)],
    pub about: &'a str,
    pub needs: Ask,
    pub cost: Option<String>,
}

/// Writes a new open ask and returns it.
pub fn make(home: &Path, n: &New) -> Result<Value, String> {
    let made = now_ms();
    let k = key(n.command, n.argv);
    let id = format!("{}-{}", made, &k[..8]);
    let game = n
        .values
        .iter()
        .find(|(k, _)| matches!(k.as_str(), "game" | "template" | "run"))
        .map(|(_, v)| v.clone())
        .filter(|g| game_ok(g));
    let mut v = json!({
        "id": id,
        "command": n.command,
        "argv": n.argv,
        "key": k,
        "game": game,
        "needs": n.needs.name(),
        "says": says(n),
        "made_ms": made,
        "state": "open",
    });
    if let Some(c) = &n.cost {
        v["cost"] = json!(c);
    }
    write(home, &v)?;
    Ok(v)
}

fn value<'a>(values: &'a [(String, String)], names: &[&str]) -> Option<&'a str> {
    values
        .iter()
        .find(|(k, _)| names.contains(&k.as_str()))
        .map(|(_, v)| v.as_str())
}

/// What the ask says to the person: what it does, in words they can judge,
/// and what it publishes and where.
pub fn says(n: &New) -> String {
    let v = |names: &[&str]| value(n.values, names);
    let game = v(&["game", "template", "run"]);
    let of = game.map(|g| format!(" of {g}")).unwrap_or_default();
    let target = v(&["target", "registry", "to", "dest", "remote"]);
    let to = target.map(|t| format!(" to {t}")).unwrap_or_default();
    let main = match n.command {
        "capture background" => {
            let who = match v(&["agents"]).unwrap_or("1") {
                "1" => "a clip agent".to_owned(),
                k => format!("up to {k} clip agents"),
            };
            format!(
                "Let {who} work on the clips{of} the tools are stuck on. Each one is a Claude session that costs money on your account while it runs, up to an hour each."
            )
        }
        "theme generate" => format!(
            "Pay an image service (Runway) to draw the pictures for a theme{of}. The pictures are made from our own words, never the game's art."
        ),
        "model gate" => format!(
            "Land the changed model{of} in the game's folder, once it passes the game's checks. Everyone who uses the game gets it."
        ),
        "registry publish" => format!(
            "Publish{} in the registry{to}. Anyone who reads that registry can get it.",
            v(&["dir", "package", "path"])
                .map(|d| format!(" {d}"))
                .unwrap_or_default()
        ),
        "registry reindex" => format!("Rewrite the public index of the registry{to}."),
        "theme publish" | "theme recheck" => format!(
            "Publish the theme{} to the public list of themes{to}.",
            v(&["theme", "dir", "name"])
                .map(|d| format!(" {d}"))
                .unwrap_or_default()
        ),
        "game attest" => format!("Sign and publish a record that the game{of} plays as checked."),
        "game fetch" | "theme fetch" | "testroms fetch" | "testroms run" => format!(
            "Download files from the internet{}. It can be large.",
            v(&["name", "game", "theme", "url"])
                .map(|d| format!(": {d}"))
                .unwrap_or_default()
        ),
        "agent integrate" => "Land a clip agent's work in the game's code.".to_owned(),
        "self publish" => "Publish this build of Game Printer for others to install.".to_owned(),
        "tool gate" => "Land a change to Game Printer itself once it passes its checks.".to_owned(),
        "dataset grade" => {
            "Read the answer key of a test set, which the work is graded against.".to_owned()
        }
        _ => {
            let about = n.about.trim().trim_end_matches('.');
            let lead = match n.needs {
                Ask::Publish => "Publish or land something others get",
                Ask::Spend => "Spend money",
                Ask::Download => "Download a lot",
            };
            if about.is_empty() {
                format!("{lead}.")
            } else {
                format!("{lead}: {about}.")
            }
        }
    };
    match &n.cost {
        Some(c) => format!("{main} Cost: {c}."),
        None => main,
    }
}

/// The cost a dry run reported, in words, when its JSON has one (theme
/// generate's `dollars` and `to_make`).
pub fn cost_of(dry: &Value) -> Option<String> {
    let dollars = dry["dollars"].as_f64()?;
    let calls = dry["to_make"].as_u64();
    Some(match calls {
        Some(k) => format!("about ${dollars:.2} for {k} pictures"),
        None => format!("about ${dollars:.2}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new<'a>(argv: &'a [String], values: &'a [(String, String)]) -> New<'a> {
        New {
            command: "registry publish",
            argv,
            values,
            about: "Publish a package",
            needs: Ask::Publish,
            cost: None,
        }
    }

    #[test]
    fn a_yes_is_used_once_and_a_no_keeps_refusing() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        let argv = vec!["--target".to_owned(), "/tmp/r".to_owned()];
        let vals = vec![("target".to_owned(), "/tmp/r".to_owned())];
        let k = key("registry publish", &argv);
        assert_eq!(find(home, None, &k), Found::None);
        let a = make(home, &new(&argv, &vals)).unwrap();
        let id = a["id"].as_str().unwrap();
        assert!(a["says"].as_str().unwrap().contains("to /tmp/r"));
        assert!(matches!(find(home, Some(id), &k), Found::Open(_)));
        assert_eq!(pending(home).len(), 1);
        answer(home, id, Answer::Yes).unwrap();
        assert!(answer(home, id, Answer::No).is_err(), "answered once");
        assert!(matches!(find(home, Some(id), &k), Found::Yes(_)));
        assert_eq!(find(home, Some(id), &k), Found::None, "used");
        assert!(pending(home).is_empty());
        // Another ask, answered no.
        let b = make(home, &new(&argv, &vals)).unwrap();
        let id = b["id"].as_str().unwrap();
        answer(home, id, Answer::No).unwrap();
        assert!(matches!(find(home, Some(id), &k), Found::No(_)));
        assert!(matches!(find(home, None, &k), Found::No(_)));
        // A changed argument is another call.
        let other = key("registry publish", &["--target".into(), "/tmp/s".into()]);
        assert!(matches!(find(home, Some(id), &other), Found::Other(_)));
        assert_eq!(find(home, None, &other), Found::None);
    }
}
