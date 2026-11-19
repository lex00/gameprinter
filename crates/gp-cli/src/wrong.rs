//! WRONG (docs/loop.md, "Wrong"): the person marks one of the loop's open
//! steps wrong, and the loop halts. An objection is
//! `$GP_HOME/work/loop/wrong/ID.json`: its own id (`w` and 12 hex
//! digits), the step as it was listed, the person's words, when, and the
//! facts the step was decided from, so a test can pin them. While an
//! objection is open, `next` emits no steps (it says the loop is halted,
//! by which step and why), gp_run runs no step and the runner starts no
//! worker.
//!
//! The rule that clears it: a commit on the tool's branch that adds a line
//! naming the objection's id, which is a test or a fixture that pins the
//! rule change. The tool gate checks each patch it lands
//! ([`clear_landed`]); `gp loop resume ID` checks the branch's history
//! for one, for a change that landed before the objection was read.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use crate::next::{Answer, Who, step};

fn dir(home: &Path) -> std::path::PathBuf {
    gp_mcp::steps::loop_dir(home).join("wrong")
}

fn id_ok(id: &str) -> bool {
    id.len() == 13 && id.starts_with('w') && id[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Every objection, oldest first.
pub(crate) fn all(home: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = std::fs::read_dir(dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice::<Value>(&b).ok())
        .collect();
    out.sort_by_key(|v| v["at_ms"].as_u64().unwrap_or(0));
    out
}

/// The objection that halts the loop: the oldest still open.
pub(crate) fn halt(home: &Path) -> Option<Value> {
    all(home).into_iter().find(|v| v["state"] == json!("open"))
}

fn write(home: &Path, v: &Value) -> Result<()> {
    let d = dir(home);
    std::fs::create_dir_all(&d)?;
    let id = v["id"].as_str().unwrap_or("");
    let f = d.join(format!("{id}.json"));
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, &f)?;
    Ok(())
}

/// Records the person's objection to `step` (as `gp loop steps` lists
/// it), decided from `facts`.
pub(crate) fn record(home: &Path, step: &Value, facts: &Value, why: &str) -> Result<Value> {
    let why = why.trim();
    if why.is_empty() {
        bail!("say what is wrong with the step");
    }
    let at_ms = gp_mcp::asks::now_ms();
    let key = format!("{}\n{at_ms}\n{why}", step["id"].as_str().unwrap_or(""));
    let id = format!("w{}", &blake3::hash(key.as_bytes()).to_hex()[..12]);
    let v = json!({
        "id": id,
        "state": "open",
        "step": step,
        "why": why,
        "at_ms": at_ms,
        "facts": facts,
    });
    write(home, &v)?;
    Ok(v)
}

/// Marks the open step `step_id` wrong, with the facts its answer was
/// decided from. Only an open step, or one a live worker holds, can be.
pub(crate) fn mark(
    paths: &gp_evidence::Paths,
    games: Option<&Path>,
    step_id: &str,
    why: &str,
) -> Result<Value> {
    let answers = crate::steps::answers(paths, games);
    for a in &answers {
        if let Some(s) = a.open.iter().find(|s| s.id == step_id) {
            return record(&paths.home, &crate::steps::listed(a, s), &a.facts, why);
        }
    }
    if let Some(s) = gp_mcp::steps::held(&paths.home)
        .into_iter()
        .find(|s| s["id"] == json!(step_id))
    {
        return record(&paths.home, &s, &json!(null), why);
    }
    bail!("no open step {step_id} (gp loop steps lists the open ones)")
}

fn clear(home: &Path, mut v: Value, commit: &str, by: &str) -> Result<Value> {
    v["state"] = json!("cleared");
    v["cleared"] = json!({"commit": commit, "by": by, "at_ms": gp_mcp::asks::now_ms()});
    write(home, &v)?;
    Ok(v)
}

/// After the tool gate lands `commit`: each open objection whose id a
/// line the commit adds names is cleared. Returns a line for each.
pub(crate) fn clear_landed(home: &Path, repo: &Path, commit: &str) -> Vec<String> {
    let open: Vec<Value> = all(home)
        .into_iter()
        .filter(|v| v["state"] == json!("open"))
        .collect();
    if open.is_empty() {
        return Vec::new();
    }
    let Ok(diff) = crate::gate::git(repo, &["show", "--format=", "--unified=0", commit]) else {
        return Vec::new();
    };
    let added: Vec<&str> = diff
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .collect();
    let mut out = Vec::new();
    for v in open {
        let id = v["id"].as_str().unwrap_or("").to_owned();
        if !id.is_empty()
            && added.iter().any(|l| l.contains(&id))
            && clear(home, v, commit, "gate").is_ok()
        {
            out.push(format!(
                "the loop resumes: the change names objection {id}, which halted it"
            ));
        }
    }
    out
}

/// `gp loop resume ID`: clears objection `id` when a commit on `branch`
/// of `repo` adds a line naming it.
pub(crate) fn resume(home: &Path, repo: &Path, branch: &str, id: &str) -> Result<Value> {
    if !id_ok(id) {
        bail!("an objection's id is w and 12 hex digits (got {id:?})");
    }
    let v = all(home)
        .into_iter()
        .find(|v| v["id"] == json!(id))
        .with_context(|| format!("no objection {id}"))?;
    if v["state"] != json!("open") {
        bail!("objection {id} is cleared already");
    }
    let found = crate::gate::git(repo, &["log", "--format=%H", "-S", id, branch])?;
    let Some(commit) = found.lines().next().filter(|c| !c.is_empty()) else {
        bail!(
            "no commit on {branch} adds a line naming {id}: land the rule change with a test that names it through gp tool gate, which clears it"
        );
    };
    clear(home, v, commit, "resume")
}

/// The halt in plain words.
pub(crate) fn says(v: &Value) -> String {
    let step = &v["step"];
    format!(
        "The loop is halted: you marked step {} wrong ({}): \"{}\" No step starts until a change to the tool that names {} (a test that pins it) lands through gp tool gate.",
        step["id"].as_str().unwrap_or("?"),
        crate::next::cut(step["says"].as_str().unwrap_or(""), 80),
        v["why"].as_str().unwrap_or("").trim(),
        v["id"].as_str().unwrap_or("?"),
    )
}

/// The halt as the answer's `halted`.
pub(crate) fn halted_json(v: &Value) -> Value {
    json!({
        "id": v["id"],
        "step": v["step"]["id"],
        "why": v["why"],
        "at_ms": v["at_ms"],
        "says": says(v),
    })
}

/// An answer while the loop is halted: no step open, and the halt as its
/// next.
pub(crate) fn apply(a: &mut Answer, halt: Option<&Value>) {
    let Some(h) = halt else { return };
    a.open.clear();
    a.also.clear();
    a.next = step(Who::Person, says(h));
    a.halted = Some(halted_json(h));
}

/// [`apply`] with the home's halt.
pub(crate) fn apply_home(home: &Path, a: &mut Answer) {
    apply(a, halt(home).as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_in(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn a_step() -> Value {
        json!({"id": "s111111111111", "journey": "port", "subject": "g", "who": "agent",
            "says": "Your agent can add 2 new clips to the game's tests.", "commands": ["corpus import"],
            "touches": ["game:g"]})
    }

    #[test]
    fn an_objection_halts_the_loop_with_its_facts_until_a_landing_names_it() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let repo = t.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git_in(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git_in(&repo, &["add", "."]);
        git_in(&repo, &["commit", "-qm", "first"]);

        assert!(halt(&home).is_none());
        let facts = json!({"not_kept": ["jump 1", "jump 2"], "corpus_clips": 4});
        assert!(
            record(&home, &a_step(), &facts, "  ").is_err(),
            "words are needed"
        );
        let w = record(
            &home,
            &a_step(),
            &facts,
            "it adds clips the person threw away",
        )
        .unwrap();
        let id = w["id"].as_str().unwrap().to_owned();
        assert!(id_ok(&id), "{id}");
        // The record pins the step and the facts it was decided from.
        let h = halt(&home).unwrap();
        assert_eq!(h["step"]["id"], "s111111111111");
        assert_eq!(h["facts"], facts);
        assert_eq!(h["why"], "it adds clips the person threw away");

        // A halted answer opens nothing and says why.
        let mut a = crate::next::answer(&crate::next::Facts::default());
        assert!(!a.open.is_empty());
        apply(&mut a, Some(&h));
        assert!(a.open.is_empty() && a.also.is_empty());
        assert_eq!(a.next.who, Who::Person);
        assert!(
            a.next
                .says
                .starts_with("The loop is halted: you marked step s111111111111 wrong"),
            "{}",
            a.next.says
        );
        assert!(a.next.says.contains(&id), "{}", a.next.says);
        assert_eq!(a.halted.as_ref().unwrap()["id"], json!(id));

        // A landing that doesn't name it leaves the halt; resume finds no commit.
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        git_in(&repo, &["add", "."]);
        git_in(&repo, &["commit", "-qm", "unrelated"]);
        let head = crate::gate::git(&repo, &["rev-parse", "HEAD"]).unwrap();
        assert!(clear_landed(&home, &repo, &head).is_empty());
        assert!(resume(&home, &repo, "main", &id).is_err());
        assert!(halt(&home).is_some());

        // A landing whose test names it clears it.
        std::fs::write(repo.join("pin.rs"), format!("// pins objection {id}\n")).unwrap();
        git_in(&repo, &["add", "."]);
        git_in(&repo, &["commit", "-qm", "the rule change and its test"]);
        let head = crate::gate::git(&repo, &["rev-parse", "HEAD"]).unwrap();
        let lines = clear_landed(&home, &repo, &head);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(halt(&home).is_none());
        let v = all(&home).pop().unwrap();
        assert_eq!(v["cleared"]["commit"], json!(head));
        assert_eq!(v["cleared"]["by"], "gate");
    }

    #[test]
    fn resume_clears_an_objection_a_landed_commit_names() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let repo = t.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git_in(&repo, &["init", "-q", "-b", "main"]);
        let w = record(&home, &a_step(), &json!({}), "wrong row").unwrap();
        let id = w["id"].as_str().unwrap().to_owned();
        std::fs::write(repo.join("pin.rs"), format!("// {id}\n")).unwrap();
        git_in(&repo, &["add", "."]);
        git_in(&repo, &["commit", "-qm", "pin"]);
        let v = resume(&home, &repo, "main", &id).unwrap();
        assert_eq!(v["state"], "cleared");
        assert_eq!(v["cleared"]["by"], "resume");
        assert!(halt(&home).is_none());
        assert!(resume(&home, &repo, "main", &id).is_err(), "once");
    }
}
