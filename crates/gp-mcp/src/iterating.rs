//! A quest-log row the person is still iterating on (`iterating` on the
//! row, from `gp mechanic queue GAME ROW --iterating WHY` or the card's
//! LOOKS GOOD, KEEP GOING): no worker gates it, dry run or card, until the
//! person presses LAND IT on the row. On 2026-11-23 the owner answered
//! "looks good" on a theme's rows worked by hand in a working copy, workers
//! cut patches from that copy piece by piece and put up gate cards, and a
//! YES began a landing of 25 to 30 minutes per piece: "pressing YES is not
//! obvious to me that it's doing anything other than confirming its what i
//! want for this dev round ... most of the time i want to iterate fast"
//! (docs/loop.md, a gate's card).

use std::path::Path;

use serde_json::Value;

/// The gates a row marked iterating refuses.
pub fn is_gate(command: &str) -> bool {
    matches!(command, "model gate" | "tool gate")
}

/// The quest-log row step `step` is for (`row:GAME/ROW` of its worker
/// record's touches), or None.
fn step_row(home: &Path, step: &str) -> Option<(String, String)> {
    let ok = !step.is_empty() && step.chars().all(|c| c.is_ascii_alphanumeric());
    if !ok {
        return None;
    }
    let f = crate::steps::workers_dir(home).join(format!("{step}.json"));
    let w: Value = serde_json::from_slice(&std::fs::read(f).ok()?).ok()?;
    let (g, r) = w["step"]["touches"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|t| t.strip_prefix("row:"))?
        .split_once('/')?;
    Some((g.to_owned(), r.to_owned()))
}

/// Game `game`'s quest log in `home`; empty when there's none.
fn rows(home: &Path, game: &str) -> Vec<Value> {
    if !crate::asks::game_ok(game) {
        return Vec::new();
    }
    let f = home.join("mechanics").join(game).join("queue.json");
    std::fs::read(f)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Whether patch file `file` (a path its `diff --git` names) is under
/// `under`, a folder of the games checkout (rooftop/themes/night-shift) or of
/// the game's own folder (themes/night-shift).
fn is_under(file: &str, under: &str, game: &str) -> bool {
    let under = under.trim().trim_matches('/');
    if under.is_empty() {
        return false;
    }
    let inside = |f: &str| f == under || f.starts_with(&format!("{under}/"));
    inside(file) || inside(&format!("{game}/{file}"))
}

/// Why gp `command` (with its values) may not run from step `step`, or
/// None: it is a gate, and the step's row is marked iterating, or (a model
/// gate) its patch changes a file under a folder an iterating row of the
/// game names as its working copy (`iterating_on`).
pub fn refused(
    home: &Path,
    command: &str,
    step: Option<&str>,
    values: &[(String, String)],
) -> Option<String> {
    if !is_gate(command) {
        return None;
    }
    let what = |g: &str, r: &Value| {
        let row = r["mechanic"].as_str().unwrap_or("?");
        let why = r["iterating"]
            .as_str()
            .unwrap_or("")
            .trim()
            .trim_end_matches('.');
        let why = if why.is_empty() {
            String::new()
        } else {
            format!(" ({why})")
        };
        format!(
            "gp {command}: row {g}/{row} is marked iterating{why}. The person is still trying it, and nothing of it is gated, dry run or card, until they press LAND IT on the row (it then goes to next, to be gated as one patch). Leave the work where it is, say on the row what is ready (gp mechanic say), and park it: gp mechanic queue {g} {row} --status \"in line\" --parked \"iterating, waits for the person's LAND IT\""
        )
    };
    if let Some((g, r)) = step.and_then(|s| step_row(home, s))
        && let Some(row) = rows(home, &g)
            .into_iter()
            .find(|x| x["mechanic"].as_str() == Some(&r))
        && row["iterating"].is_string()
    {
        return Some(what(&g, &row));
    }
    if command != "model gate" {
        return None;
    }
    let game = crate::asks::value_of(values, "game")?;
    let files = crate::asks::patch_files(crate::asks::value_of(values, "patch")?);
    rows(home, game).into_iter().find_map(|row| {
        let on = row["iterating_on"].as_array()?;
        row["iterating"].as_str()?;
        let hit = files.iter().find(|f| {
            on.iter()
                .filter_map(Value::as_str)
                .any(|p| is_under(f, p, game))
        })?;
        Some(format!(
            "{}; its patch changes {hit}, in the working copy that row names",
            what(game, &row)
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn home_with(rows: Value) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        let w = crate::steps::workers_dir(h);
        std::fs::create_dir_all(&w).unwrap();
        std::fs::write(
            w.join("s000000000001.json"),
            json!({"step": {"id": "s000000000001", "touches": ["row:rooftop/tip-money"]}})
                .to_string(),
        )
        .unwrap();
        let q = h.join("mechanics").join("rooftop");
        std::fs::create_dir_all(&q).unwrap();
        std::fs::write(q.join("queue.json"), rows.to_string()).unwrap();
        t
    }

    fn vals(patch: &Path) -> Vec<(String, String)> {
        vec![
            ("game".into(), "rooftop".into()),
            ("patch".into(), patch.display().to_string()),
        ]
    }

    /// The owner's test: a row marked iterating yields no gate card (nor
    /// a dry run) from its worker after a "looks good"; unmarked, it does.
    #[test]
    fn a_row_marked_iterating_yields_no_gate_card() {
        let t = home_with(json!([{"mechanic": "tip-money", "status": "next",
            "iterating": "you said LOOKS GOOD, KEEP GOING on its card"}]));
        let h = t.path();
        let p = h.join("x.patch");
        std::fs::write(&p, "").unwrap();
        for cmd in ["model gate", "tool gate"] {
            let why = refused(h, cmd, Some("s000000000001"), &vals(&p)).expect(cmd);
            assert!(
                why.contains("rooftop/tip-money is marked iterating"),
                "{why}"
            );
            assert!(why.contains("LAND IT"), "{why}");
        }
        // Not a gate, or another step's row: it goes on.
        assert!(refused(h, "mechanic say", Some("s000000000001"), &vals(&p)).is_none());
        assert!(refused(h, "model gate", Some("s000000000002"), &vals(&p)).is_none());
        // The mark taken off (LAND IT): the gate goes on.
        let t = home_with(json!([{"mechanic": "tip-money", "status": "next"}]));
        assert!(refused(t.path(), "model gate", Some("s000000000001"), &vals(&p)).is_none());
    }

    /// A model gate from any row whose patch changes the working copy an
    /// iterating row names is refused too: the pieces of a theme aren't
    /// cut from it one by one.
    #[test]
    fn a_patch_of_an_iterating_rows_working_copy_is_refused() {
        let t = home_with(json!([
            {"mechanic": "tip-money", "status": "next"},
            {"mechanic": "night-shift-theme", "status": "in line",
             "iterating": "the owner works it by hand", "iterating_on": ["rooftop/themes/night-shift"]},
        ]));
        let h = t.path();
        let p = h.join("x.patch");
        std::fs::write(&p, "diff --git a/rooftop/themes/night-shift/theme/look.txt b/rooftop/themes/night-shift/theme/look.txt\n").unwrap();
        let why = refused(h, "model gate", Some("s000000000001"), &vals(&p)).expect("refused");
        assert!(why.contains("rooftop/night-shift-theme"), "{why}");
        assert!(
            why.contains("rooftop/themes/night-shift/theme/look.txt"),
            "{why}"
        );
        std::fs::write(&p, "diff --git a/rooftop/model.toml b/rooftop/model.toml\n").unwrap();
        assert!(refused(h, "model gate", Some("s000000000001"), &vals(&p)).is_none());
        assert!(is_under(
            "themes/night-shift/a.png",
            "rooftop/themes/night-shift",
            "rooftop"
        ));
        assert!(!is_under(
            "rooftop/themes/night-shift-2/a.png",
            "rooftop/themes/night-shift",
            "rooftop"
        ));
    }
}
