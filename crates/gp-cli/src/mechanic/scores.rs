//! The quest log with the gate's scores: a row whose clips are in the
//! corpus takes its status and a line of scores from the status file the
//! gate writes when it lands a patch (`$GP_HOME/work/refine/GAME/status.json`,
//! `crate::gate::write_status`).
//!
//! Ported from game-printer's mechanic.rs (`scored_queue`).

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::session::read_queue;

/// The quest log as shown: [`read_queue`] with each row's scores from the
/// gate's status file when its clips are in the corpus. Such a row's
/// `status` is `done` when every clip meets the contract's bars (100% of
/// frames together where the contract sets none), and `scores` says the
/// score, the first split and what differs; the typed status stays as
/// `typed_status` and the typed note as `note`. A row out of scope, a
/// sealed row, or a row with no clip in the corpus, is shown as typed.
pub(crate) fn scored_queue(home: &Path, game: &str) -> Vec<Value> {
    let mut q = read_queue(&home.join("mechanics").join(game).join("queue.json"));
    let status: Value = fs::read(
        home.join("work")
            .join("refine")
            .join(game)
            .join("status.json"),
    )
    .ok()
    .and_then(|b| serde_json::from_slice(&b).ok())
    .unwrap_or(Value::Null);
    for r in &mut q {
        let Some(m) = r["mechanic"].as_str().map(str::to_owned) else {
            continue;
        };
        if r["scope"] == json!("out") || m.starts_with("sealed") {
            continue;
        }
        let clips: Vec<&Value> = status["clips"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["mechanic"] == json!(m))
            .collect();
        if clips.is_empty() {
            continue;
        }
        let in_session = fs::read_dir(home.join("mechanics").join(game).join(&m).join("clips"))
            .map(|d| {
                d.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
                    .count()
            })
            .unwrap_or(0);
        r["typed_status"] = r["status"].clone();
        let (line, all) = scores_line(&status, &m, &clips, in_session);
        r["scores"] = json!(line);
        r["status"] = json!(if all {
            "done"
        } else if r["typed_status"] == json!("done") {
            "next"
        } else {
            r["typed_status"].as_str().unwrap_or("next")
        });
    }
    q
}

/// Whether a clip of the status file meets the bars: those the contract
/// set, else 100% of its frames together straight through.
pub(crate) fn meets(c: &Value) -> bool {
    let b = &c["bars"];
    if b.is_object() && (b["together"].is_boolean() || b["longest_stretch"].is_boolean()) {
        return b["together"] != json!(false) && b["longest_stretch"] != json!(false);
    }
    c["straight"]["together"].as_u64().is_some() && c["straight"]["together"] == c["straight"]["of"]
}

/// A row's line from the gate's scores, and whether every clip meets the
/// bars.
fn scores_line(status: &Value, m: &str, clips: &[&Value], in_session: usize) -> (String, bool) {
    let met = clips.iter().filter(|c| meets(c)).count();
    let row = status["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["mechanic"] == json!(m));
    let pct = |v: &Value| v["pct"].as_f64().map(|p| format!("{p:.1}%"));
    let mut parts = vec![format!("{met} of {} clips meet the bars", clips.len())];
    if let Some(r) = row {
        if let Some(p) = pct(&r["straight"]) {
            parts.push(format!("{p} straight through"));
        }
        if let Some(p) = pct(&r["resync"]) {
            parts.push(format!("{p} with resync"));
        }
        if let Some(l) = r["longest_exact"]["frames"].as_u64() {
            parts.push(format!(
                "longest exact stretch {} s",
                gp_interp::lockstep::seconds(l)
            ));
        }
    }
    // The first split of the first clip below the bars.
    if let Some(c) = clips.iter().find(|c| !meets(c))
        && let Some(f) = c["first_split"].as_object()
    {
        parts.push(format!(
            "first split: clip {} at frame {} (the clip's {}), {}",
            c["clip"],
            f.get("frame").cloned().unwrap_or(Value::Null),
            f.get("clip_frame").cloned().unwrap_or(Value::Null),
            f.get("what").and_then(Value::as_str).unwrap_or("")
        ));
    }
    if in_session > clips.len() {
        let n = in_session - clips.len();
        parts.push(format!(
            "{n} clip{} not in the corpus yet",
            if n == 1 { "" } else { "s" }
        ));
    }
    let model = status["model"]["commit"].as_str().unwrap_or("");
    (
        format!("{} (gate, model {model})", parts.join("; ")),
        met == clips.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row with clips in the corpus shows the gate's scores and takes
    /// its status from them; the typed note stays; a row out of scope or
    /// with no scored clip stays as typed.
    #[test]
    fn the_quest_log_takes_its_scores_from_the_gates_status() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path().join("mechanics/dk");
        for (m, n) in [("walk", 1), ("clear", 2)] {
            fs::create_dir_all(g.join(m).join("clips")).unwrap();
            for i in 1..=n {
                fs::write(g.join(m).join(format!("clips/{i:03}.toml")), "start = 0\n").unwrap();
            }
        }
        fs::write(
            g.join("queue.json"),
            r#"[{"mechanic": "walk", "status": "agent working", "note": "typed: 4 of 4"},
                {"mechanic": "clear", "status": "done", "note": "typed: waits on the hammer"},
                {"mechanic": "blind-farm", "status": "next", "scope": "out", "scope_why": "the blind farm"},
                {"mechanic": "jump", "status": "next", "note": "not recorded"}]"#,
        )
        .unwrap();
        let r = d.path().join("work/refine/dk");
        fs::create_dir_all(&r).unwrap();
        fs::write(
            r.join("status.json"),
            serde_json::to_string(&json!({
                "model": {"commit": "07701d77"},
                "rows": [
                    {"mechanic": "walk", "straight": {"together": 352, "of": 352, "pct": 100.0}, "resync": null, "longest_exact": null},
                    {"mechanic": "clear", "straight": {"together": 3000, "of": 3360, "pct": 89.3},
                     "resync": {"together": 3300, "of": 3360, "pct": 98.2}, "longest_exact": {"frames": 1640, "clip": 1}}],
                "clips": [
                    {"id": "dk.walk.001", "mechanic": "walk", "clip": 1, "straight": {"together": 352, "of": 352},
                     "first_split": null, "bars": {"together": true, "longest_stretch": null}},
                    {"id": "dk.clear-25m.001", "mechanic": "clear", "clip": 1, "straight": {"together": 3000, "of": 3360},
                     "first_split": {"frame": 3446, "clip_frame": 3000, "what": "mario.x apart"},
                     "bars": {"together": false, "longest_stretch": true}},
                    {"id": "dk.blind-farm.001", "mechanic": "blind-farm", "clip": 1, "straight": {"together": 1, "of": 1}}]
            }))
            .unwrap(),
        )
        .unwrap();
        let q = scored_queue(d.path(), "dk");
        assert_eq!(q[0]["status"], "done");
        assert_eq!(q[0]["typed_status"], "agent working");
        assert_eq!(q[0]["note"], "typed: 4 of 4");
        assert_eq!(
            q[0]["scores"],
            "1 of 1 clips meet the bars; 100.0% straight through (gate, model 07701d77)"
        );
        // Typed done, but below the bars: back in the queue.
        assert_eq!(q[1]["status"], "next");
        let s = q[1]["scores"].as_str().unwrap();
        assert!(
            s.starts_with("0 of 1 clips meet the bars; 89.3% straight through; 98.2% with resync; longest exact stretch 27.3 s"),
            "{s}"
        );
        assert!(
            s.contains("first split: clip 1 at frame 3446 (the clip's 3000), mario.x apart"),
            "{s}"
        );
        assert!(s.contains("1 clip not in the corpus yet"), "{s}");
        assert_eq!(q[1]["note"], "typed: waits on the hammer");
        assert!(q[2]["scores"].is_null() && q[2]["status"] == "next");
        assert!(q[3]["scores"].is_null() && q[3]["status"] == "next");
    }
}
