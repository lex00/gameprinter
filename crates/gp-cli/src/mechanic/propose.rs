//! `gp mechanic propose`: the loop adds a quest-log row on its own. What
//! the picture check finds (`gp picture --propose`) and what a worker finds
//! while it works become rows with no card: status next, a note that
//! starts "Proposed by the loop:" and says where it came from. A worker
//! that proposes a row gives its test too (`--goal`), written to the
//! game's triage.toml in the same call, so the triage tries the new row
//! before an agent or the person does (docs/loop.md).

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use serde_json::{Value, json};

use super::session::{name_ok, queue_file, read_queue, write_queue};

/// The words every proposed row's note starts with.
pub(crate) const MARK: &str = "Proposed by the loop:";

/// A row the loop proposes.
#[derive(Default)]
pub(crate) struct Proposal<'a> {
    pub game: &'a str,
    pub row: &'a str,
    /// Where it came from: the command, the clip and frame, the step.
    pub from: &'a str,
    /// What is wrong or missing, in one line.
    pub what: &'a str,
    /// tools for a row of gameprinter's own work.
    pub repo: Option<&'a str>,
    /// The row's triage test, with the person's request.
    pub test: Option<Test<'a>>,
    /// The clips the gap is seen in (`gp picture --propose`). A row the
    /// quest log has already gets each clip it doesn't name yet added to its
    /// note and its `seen_in`, and goes back to next if it was done: the
    /// row is done only when every clip it names is fixed.
    pub clips: &'a [String],
}

/// Whether the row names the clip: in `seen_in`, or (rows from before
/// `seen_in`) in its note or where it came from.
fn names_clip(r: &Value, clip: &str) -> bool {
    r["seen_in"]
        .as_array()
        .is_some_and(|a| a.iter().any(|c| c == clip))
        || r["note"].as_str().is_some_and(|s| s.contains(clip))
        || r["from"].as_str().is_some_and(|s| s.contains(clip))
}

/// A triage.toml item's test and request (mechanic/triage.rs has the
/// format).
#[derive(Default)]
pub(crate) struct Test<'a> {
    pub goal: &'a str,
    pub near: Option<&'a str>,
    pub from: Option<&'a str>,
    pub place: Option<&'a str>,
    pub todo: Option<&'a str>,
    pub stop: Option<&'a str>,
}

/// A row name from words: lower case, a dash between words, at most `max`
/// bytes, cut at a dash.
pub(crate) fn slug(words: &str, max: usize) -> String {
    let mut s = String::new();
    for w in words
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if !s.is_empty() && s.len() + 1 + w.len() > max {
            break;
        }
        if !s.is_empty() {
            s.push('-');
        }
        s.push_str(w);
    }
    s.truncate(max);
    s
}

/// Adds the row unless the quest log has it (any status: a row the person
/// finished or put out of scope stays so), and its test to triage.toml in
/// `game_dir` unless an item names the row. The test is checked first, so
/// a test the triage can't read writes nothing. Says what it did.
pub(crate) fn propose(paths: &Paths, game_dir: Option<&Path>, p: &Proposal) -> Result<Value> {
    if !name_ok(p.row) {
        bail!(
            "a mechanic name is lower-case letters, digits and dashes: {}",
            p.row
        );
    }
    if p.from.trim().is_empty() || p.what.trim().is_empty() {
        bail!("a proposed row says what it is (--what) and where it came from (--from)");
    }
    let mut had_test = false;
    if let Some(t) = &p.test {
        let dir = game_dir.context("a row's test goes in the game's triage.toml: no games checkout found (pass --games or set GP_GAMES_DIR)")?;
        had_test = super::triage::check_test(dir, p.row, t.goal)?;
    }
    let f = queue_file(paths, p.game)?;
    let mut q = read_queue(&f);
    let mut at = q.iter().position(|r| r["mechanic"] == json!(p.row));
    // An archived row is still the row: proposed again it stays archived,
    // as a done row stays done, unless it was seen in a new clip, when it
    // comes back to the quest log to be reopened below.
    let mut archived = false;
    if at.is_none() {
        let is_it = |r: &Value| r["mechanic"] == json!(p.row);
        if let Some(r) = super::archive::read_archive(&paths.home, p.game)
            .into_iter()
            .find(is_it)
        {
            archived = true;
            if p.clips.iter().any(|c| !names_clip(&r, c))
                && let Some(r) = super::archive::bring_back(&paths.home, p.game, is_it)?
            {
                q.push(r);
                at = Some(q.len() - 1);
            }
        }
    }
    let had_row = at.is_some() || archived;
    let mut new_clips: Vec<&String> = Vec::new();
    let mut reopened = false;
    match at {
        None if archived => {}
        None => {
            let mut r = json!({
                "mechanic": p.row,
                "status": "next",
                "note": format!("{MARK} {} (from {})", p.what.trim(), p.from.trim()),
                "by": "loop",
                "from": p.from.trim(),
            });
            if p.repo == Some("tools") {
                r["repo"] = json!("tools");
            }
            if !p.clips.is_empty() {
                r["seen_in"] = json!(p.clips);
            }
            q.push(r);
            write_queue(&f, &q)?;
        }
        Some(i) => {
            let r = &mut q[i];
            new_clips = p.clips.iter().filter(|c| !names_clip(r, c)).collect();
            if !new_clips.is_empty() {
                let mut seen: Vec<Value> = r["seen_in"].as_array().cloned().unwrap_or_default();
                seen.extend(new_clips.iter().map(|c| json!(c)));
                r["seen_in"] = Value::Array(seen);
                let list = new_clips
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let note = r["note"].as_str().unwrap_or("").trim().to_owned();
                let also = format!("Also seen in {list} (from {}).", p.from.trim());
                r["note"] = json!(if note.is_empty() {
                    also
                } else {
                    format!("{note} {also}")
                });
                if r["status"] == json!("done") && r["scope"] != json!("out") {
                    r["status"] = json!("next");
                    reopened = true;
                }
                write_queue(&f, &q)?;
            }
        }
    }
    let mut wrote_test = None;
    if let (Some(t), false) = (&p.test, had_test) {
        let dir = game_dir.expect("checked above");
        let file = dir.join("triage.toml");
        let mut text = fs::read_to_string(&file).unwrap_or_default();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&item_toml(p.row, t));
        fs::write(&file, text).with_context(|| file.display().to_string())?;
        wrote_test = Some(file.display().to_string());
    }
    Ok(json!({
        "row": p.row,
        "added": !had_row,
        "clips_added": new_clips,
        "reopened": reopened,
        "test": wrote_test,
        "test_was_there": had_test,
    }))
}

/// One `[[item]]` of triage.toml, its strings quoted as TOML quotes them.
fn item_toml(row: &str, t: &Test) -> String {
    let q = |s: &str| toml::Value::String(s.to_owned()).to_string();
    let mut s = format!("[[item]]\nrow = {}\ngoal = {}\n", q(row), q(t.goal));
    for (k, v) in [
        ("near", t.near),
        ("from", t.from),
        ("where", t.place),
        ("do", t.todo),
        ("stop", t.stop),
    ] {
        if let Some(v) = v.filter(|v| !v.trim().is_empty()) {
            s.push_str(&format!("{k} = {}\n", q(v)));
        }
    }
    s
}

/// The line `gp mechanic propose` prints.
pub(crate) fn words(v: &Value) -> String {
    let row = v["row"].as_str().unwrap_or("");
    let added: Vec<&str> = v["clips_added"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut s = if v["added"] == json!(true) {
        format!("{row}: added to the quest log as next, proposed by the loop.")
    } else if !added.is_empty() {
        let back = if v["reopened"] == json!(true) {
            "; it was done, so it is next again"
        } else {
            ""
        };
        format!(
            "{row}: the quest log has it; {} added to its note{back}.",
            added.join(", ")
        )
    } else {
        format!("{row}: the quest log has it already; left as it is.")
    };
    if let Some(f) = v["test"].as_str() {
        s.push_str(&format!(" Its test is in {f}."));
    } else if v["test_was_there"] == json!(true) {
        s.push_str(" triage.toml has its test already.");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_paths as paths;

    #[test]
    fn a_slug_is_a_row_name() {
        assert_eq!(
            slug("Placed elsewhere: hero: 1 px further right", 36),
            "placed-elsewhere-hero-1-px-further"
        );
        assert!(name_ok(&slug("original only: tiles 2a 3b", 40)));
    }

    /// A proposed row is next with the mark and where it came from; its
    /// test lands in triage.toml once; a second proposal of the row changes
    /// nothing, and a row the person finished stays done.
    #[test]
    fn a_proposed_row_is_next_with_its_test() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let game = d.path().join("games/made-up");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("triage.toml"), "[watch]\nlives = 0x10\n").unwrap();
        let p = paths(&home);
        let prop = Proposal {
            game: "made-up",
            row: "lose-a-life",
            from: "a worker on get-ready-screen",
            what: "no clip shows a life lost",
            test: Some(Test {
                goal: "lives == 2",
                todo: Some("Lose a life"),
                stop: Some("When the next life starts"),
                ..Test::default()
            }),
            ..Proposal::default()
        };
        let v = propose(&p, Some(&game), &prop).unwrap();
        assert_eq!(v["added"], json!(true));
        let q = read_queue(&queue_file(&p, "made-up").unwrap());
        assert_eq!(q[0]["status"], "next");
        assert_eq!(
            q[0]["note"],
            "Proposed by the loop: no clip shows a life lost (from a worker on get-ready-screen)"
        );
        let t = fs::read_to_string(game.join("triage.toml")).unwrap();
        assert!(
            t.contains(
                "[[item]]\nrow = \"lose-a-life\"\ngoal = \"lives == 2\"\ndo = \"Lose a life\""
            ),
            "{t}"
        );
        assert!(super::super::triage::check_test(&game, "lose-a-life", "lives == 2").unwrap());

        let again = propose(&p, Some(&game), &prop).unwrap();
        assert_eq!(again["added"], json!(false));
        assert_eq!(again["test"], json!(null));
        assert_eq!(
            fs::read_to_string(game.join("triage.toml")).unwrap(),
            t,
            "the test is written once"
        );

        let mut q = read_queue(&queue_file(&p, "made-up").unwrap());
        q[0]["status"] = json!("done");
        write_queue(&queue_file(&p, "made-up").unwrap(), &q).unwrap();
        propose(&p, Some(&game), &prop).unwrap();
        let q = read_queue(&queue_file(&p, "made-up").unwrap());
        assert_eq!((q.len(), q[0]["status"].as_str()), (1, Some("done")));
    }

    /// A gap proposed from another clip is added to the row's note and
    /// `seen_in`, and a done row goes back to next; a clip the row names
    /// (in `seen_in` or, for an older row, its from) changes nothing.
    #[test]
    fn a_new_clip_is_added_to_its_row() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(&d.path().join("home"));
        let f = queue_file(&p, "rooftop").unwrap();
        let l1 = vec!["rooftop.l1.001".to_owned()];
        let first = Proposal {
            game: "rooftop",
            row: "picture-other-the-level-picture-main",
            from: "gp picture --run r, rooftop.l1.001 frame 0",
            what: "the picture differs",
            clips: &l1,
            ..Proposal::default()
        };
        propose(&p, None, &first).unwrap();
        assert_eq!(read_queue(&f)[0]["seen_in"], json!(["rooftop.l1.001"]));
        let mut q = read_queue(&f);
        q[0]["status"] = json!("done");
        q[0]["note"] = json!("Landed: level 1's start");
        write_queue(&f, &q).unwrap();

        let both = vec!["rooftop.demo.001".to_owned(), "rooftop.l1.001".to_owned()];
        let v = propose(
            &p,
            None,
            &Proposal {
                from: "gp picture --run r, rooftop.demo.001 frame 2",
                clips: &both,
                test: None,
                ..first
            },
        )
        .unwrap();
        assert_eq!(v["clips_added"], json!(["rooftop.demo.001"]));
        assert!(
            words(&v).contains("rooftop.demo.001 added"),
            "{}",
            words(&v)
        );
        let q = read_queue(&f);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["status"], "next");
        assert_eq!(
            q[0]["seen_in"],
            json!(["rooftop.l1.001", "rooftop.demo.001"])
        );
        assert_eq!(
            q[0]["note"],
            "Landed: level 1's start Also seen in rooftop.demo.001 (from gp picture --run r, rooftop.demo.001 frame 2)."
        );

        let again = propose(
            &p,
            None,
            &Proposal {
                clips: &both,
                test: None,
                ..first
            },
        )
        .unwrap();
        assert_eq!(again["clips_added"], json!([]));

        // A row from before seen_in names its clip in from.
        let mut q = read_queue(&f);
        q[0].as_object_mut().unwrap().remove("seen_in");
        q[0]["status"] = json!("done");
        write_queue(&f, &q).unwrap();
        propose(&p, None, &first).unwrap();
        assert_eq!(read_queue(&f)[0]["status"], "done");

        // Archived, the row stays archived when proposed again from a clip
        // it names, and comes back reopened from a new one: never made anew.
        let a = super::super::archive::archive_file(&p.home, "rooftop");
        write_queue(&a, &read_queue(&f)).unwrap();
        write_queue(&f, &[]).unwrap();
        let v = propose(&p, None, &first).unwrap();
        assert_eq!(v["added"], json!(false));
        assert!(read_queue(&f).is_empty());
        let l2 = vec!["rooftop.l2.001".to_owned()];
        propose(
            &p,
            None,
            &Proposal {
                clips: &l2,
                ..first
            },
        )
        .unwrap();
        let q = read_queue(&f);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["status"], "next");
        assert!(
            q[0]["note"]
                .as_str()
                .unwrap()
                .starts_with("Landed: level 1's start")
        );
        assert!(read_queue(&a).is_empty());
    }

    /// A test the triage can't read writes nothing: no row, no item.
    #[test]
    fn a_bad_test_writes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let game = d.path().join("games/made-up");
        fs::create_dir_all(&game).unwrap();
        let p = paths(&home);
        let prop = Proposal {
            game: "made-up",
            row: "lose-a-life",
            from: "a worker",
            what: "no clip shows it",
            test: Some(Test {
                goal: "lives is two",
                ..Test::default()
            }),
            ..Proposal::default()
        };
        assert!(propose(&p, Some(&game), &prop).is_err());
        assert!(read_queue(&queue_file(&p, "made-up").unwrap()).is_empty());
        assert!(!game.join("triage.toml").exists());
    }
}
