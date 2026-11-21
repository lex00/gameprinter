//! A game's port run keeps the gp it started on (quest log
//! tapper/pin-gp-for-port-run; docs/loop.md, "The runner"). gp serve moves
//! itself onto each newly published gp (serve/reload.rs), and before this
//! each new worker on a game started on whatever was published then, so a
//! tool row landing part way through changed the tools under a port, and
//! two people porting one ROM ran different tools for the same rows.
//!
//! The first worker the runner starts on a game's own work (a quest-log row
//! of the game's, not a tools row) writes `$GP_HOME/work/loop/runs/GAME.json`
//! naming the gp it starts on. Every later worker on the game starts on that
//! gp, and so does the YES of a card one of them raised, while the tool
//! landings publish newer ones beside it. The run ends when no row of the
//! game's own is next, agent working or asking you and no worker is on it
//! (rows in line or done, and tools rows, hold no run): the next worker
//! starts a new run on the newest gp. `gp loop set --move-on GAME` ends it by hand.
//! publish.rs keeps a run's build when it prunes old ones.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The folder of the runs' records.
pub(crate) fn dir(home: &Path) -> PathBuf {
    gp_mcp::steps::loop_dir(home).join("runs")
}

fn file(home: &Path, game: &str) -> Option<PathBuf> {
    crate::mechanic::session::name_ok(game).then(|| dir(home).join(format!("{game}.json")))
}

/// Game `game`'s run, when one is on.
pub(crate) fn read(home: &Path, game: &str) -> Option<Value> {
    serde_json::from_slice(&fs::read(file(home, game)?).ok()?).ok()
}

/// Every run that is on, by game.
pub(crate) fn all(home: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = fs::read_dir(dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_str()?.strip_suffix(".json")?.to_owned();
            read(home, &n)
        })
        .collect();
    out.sort_by(|a, b| a["game"].as_str().cmp(&b["game"].as_str()));
    out
}

fn write(home: &Path, game: &str, v: &Value) {
    let Some(f) = file(home, game) else { return };
    let _ = fs::create_dir_all(dir(home));
    let tmp = f.with_extension("json.tmp");
    if fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &f);
    }
}

/// The game a step's worker works on as part of the game's port: a
/// quest-log row of the game's own (a tools row changes gp itself and runs
/// on the newest), or the engine's step on the game (`game:NAME`).
pub(crate) fn game_of(step: &Value) -> Option<String> {
    if step["repo"] == json!("tools") {
        return None;
    }
    let touches = step["touches"].as_array()?;
    let t = |p: &str| {
        touches
            .iter()
            .filter_map(Value::as_str)
            .find_map(|t| t.strip_prefix(p).map(str::to_owned))
    };
    t("row:")
        .and_then(|r| r.split_once('/').map(|(g, _)| g.to_owned()))
        .or_else(|| t("game:"))
}

/// The gp a worker on `game` starts on: its run's, while the run is on and
/// its build is still there; else `newest`, and a run starts on it.
pub(crate) fn gp_for(home: &Path, game: &str, newest: &Path, now_ms: u64) -> PathBuf {
    let mut run = read(home, game);
    if let Some(r) = &mut run {
        let gp = r["gp"].as_str().map(PathBuf::from);
        if let Some(gp) = gp.filter(|g| g.is_file()) {
            r["workers"] = json!(r["workers"].as_u64().unwrap_or(0) + 1);
            write(home, game, r);
            return gp;
        }
    }
    let gp = fs::canonicalize(newest).unwrap_or_else(|_| newest.to_path_buf());
    write(
        home,
        game,
        &json!({
            "game": game,
            "gp": gp,
            "started_ms": now_ms,
            "workers": 1,
            "why": if run.is_some() {
                "the run's gp was gone, so it starts again on the newest"
            } else {
                "a worker started on the game"
            },
        }),
    );
    gp
}

/// Ends game `game`'s run: its next worker starts on the newest gp. Whether
/// one was on.
pub(crate) fn end(home: &Path, game: &str) -> bool {
    file(home, game).is_some_and(|f| fs::remove_file(f).is_ok())
}

/// The statuses of a row the loop is on: a worker's to take up, a worker's
/// now, or the person's look a worker goes on from. A row in line (parked
/// for the person, or its change on a card, whose YES runs on its worker's
/// gp anyway) or done holds no run.
const AT_WORK: &[&str] = &["next", "agent working", "asking you"];

/// Whether game `game`'s port is still at work: a row of the game's own
/// (not a tools row) the loop is on ([`AT_WORK`]), or a live worker on the
/// game.
pub(crate) fn at_work(home: &Path, game: &str, workers: &[Value]) -> bool {
    let f = home.join("mechanics").join(game).join("queue.json");
    let open = crate::mechanic::session::read_queue(&f).iter().any(|r| {
        r["repo"] != json!("tools") && AT_WORK.contains(&r["status"].as_str().unwrap_or(""))
    });
    open || workers
        .iter()
        .filter(|w| gp_mcp::steps::holds(w))
        .any(|w| game_of(&w["step"]).as_deref() == Some(game))
}

/// Ends each run whose game's port is no longer at work ([`at_work`]).
/// The games whose runs ended.
pub(crate) fn end_finished(home: &Path, workers: &[Value]) -> Vec<String> {
    let mut ended = Vec::new();
    for r in all(home) {
        let Some(g) = r["game"].as_str() else {
            continue;
        };
        if !at_work(home, g, workers) && end(home, g) {
            ended.push(g.to_owned());
        }
    }
    ended
}

/// The builds the runs are on, by file name, which publish.rs keeps.
pub(crate) fn builds(home: &Path) -> BTreeSet<std::ffi::OsString> {
    all(home)
        .iter()
        .filter_map(|r| r["gp"].as_str())
        .filter_map(|g| Path::new(g).file_name().map(|n| n.to_owned()))
        .collect()
}

/// A run in words: the game, its gp and, when newer is published, that
/// its workers take it once the run ends.
pub(crate) fn says(home: &Path, r: &Value) -> String {
    let g = r["game"].as_str().unwrap_or("?");
    let gp = r["gp"].as_str().unwrap_or("");
    let name = |p: &str| {
        Path::new(p)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_owned())
    };
    let published = fs::canonicalize(home.join("bin").join("gp")).ok();
    let newer = published
        .as_ref()
        .filter(|p| p.as_path() != Path::new(gp))
        .map(|p| name(&p.to_string_lossy()));
    match newer {
        Some(n) => format!(
            "{g}'s port run stays on {} ({} worker(s)); {n} is published and its workers take it once the game's rows are done, or now with gp loop set --move-on {g}",
            name(gp),
            r["workers"].as_u64().unwrap_or(0)
        ),
        None => format!(
            "{g}'s port run is on {}, the published gp ({} worker(s))",
            name(gp),
            r["workers"].as_u64().unwrap_or(0)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(touches: &[&str], repo: &str) -> Value {
        json!({"touches": touches, "repo": repo})
    }

    #[test]
    fn a_games_rows_are_its_port_and_a_tools_row_is_not() {
        assert_eq!(
            game_of(&step(&["row:tapper/x", "game:tapper"], "games")).as_deref(),
            Some("tapper")
        );
        assert_eq!(
            game_of(&step(&["game:tapper"], "")).as_deref(),
            Some("tapper")
        );
        assert_eq!(game_of(&step(&["row:tapper/x"], "tools")), None);
        assert_eq!(game_of(&step(&["page"], "")), None);
    }

    #[test]
    fn a_worker_started_in_a_port_run_keeps_the_runs_gp_after_a_tool_landing() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let bin = home.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let old = bin.join("gp-aaaaaaaaaaaa");
        let new = bin.join("gp-bbbbbbbbbbbb");
        fs::write(&old, "old").unwrap();
        fs::write(&new, "new").unwrap();
        let old = fs::canonicalize(&old).unwrap();
        let new = fs::canonicalize(&new).unwrap();
        // The first worker on tapper starts the run on the gp published then.
        assert_eq!(gp_for(home, "tapper", &old, 1), old);
        // A tool landing publishes another; the run's next worker keeps the
        // run's, and another game's first worker takes the newest.
        assert_eq!(gp_for(home, "tapper", &new, 2), old);
        assert_eq!(gp_for(home, "rooftop", &new, 3), new);
        assert_eq!(read(home, "tapper").unwrap()["workers"], json!(2));
        assert!(builds(home).contains(old.file_name().unwrap()));
        // A row still open keeps the run on; with every row of the game's
        // done (a tools row doesn't count) it ends, and the next worker
        // starts a new run on the newest.
        let q = home.join("mechanics").join("tapper");
        fs::create_dir_all(&q).unwrap();
        let rows = |s: &str| {
            json!([
                {"mechanic": "a", "status": s},
                {"mechanic": "b", "status": "next", "repo": "tools"},
            ])
        };
        fs::write(q.join("queue.json"), rows("next").to_string()).unwrap();
        // rooftop has no row open, so its run ends; tapper's stays on.
        assert_eq!(end_finished(home, &[]), vec!["rooftop".to_owned()]);
        assert!(read(home, "tapper").is_some());
        fs::write(q.join("queue.json"), rows("asking you").to_string()).unwrap();
        assert!(at_work(home, "tapper", &[]));
        // A row parked in line, waiting on the person, holds no run.
        fs::write(q.join("queue.json"), rows("in line").to_string()).unwrap();
        assert!(!at_work(home, "tapper", &[]));
        let working = json!({"state": "working", "pid": std::process::id(), "step": {"touches": ["row:tapper/a", "game:tapper"]}});
        fs::write(q.join("queue.json"), rows("done").to_string()).unwrap();
        assert!(at_work(home, "tapper", std::slice::from_ref(&working)));
        assert_eq!(end_finished(home, &[]), vec!["tapper".to_owned()]);
        assert_eq!(gp_for(home, "tapper", &new, 4), new);
        // Its build gone, a run starts again on the newest.
        assert!(end(home, "tapper"));
        assert_eq!(gp_for(home, "tapper", &old, 5), old);
        fs::remove_file(&old).unwrap();
        assert_eq!(gp_for(home, "tapper", &new, 6), new);
        assert!(
            read(home, "tapper").unwrap()["why"]
                .as_str()
                .unwrap()
                .contains("gone")
        );
    }
}
