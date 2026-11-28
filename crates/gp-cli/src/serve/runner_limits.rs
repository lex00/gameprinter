//! The runner's limits (issue 43; docs/loop.md, "The runner's limits"):
//! what holds gp serve's runner (serve/runner.rs) from starting a worker,
//! and what stops one at work.
//!
//! - A step whose workers failed `fail_tries` tries (loop.toml, 3 by
//!   default) becomes a trouble row of the loop's own with the tries'
//!   endings (crate::trouble, kind `worker`), and its row waits in line on
//!   that trouble row, so no worker takes it up until the trouble row is
//!   done or the row is set next again. The tries are counted from the runs
//!   ledger (crate::worker_runs), from the step's last give-up on.
//! - A day's spend: past `day_spend_usd` over all the runs of the day in the
//!   ledger, the runner starts no worker until the next day.
//! - The machine's load: over `max_load` (the cores when 0), read as gp
//!   worker env reads it (crate::worker::Load), the runner starts no new
//!   worker.
//! - The transcript watch: a worker's stream-json log, read as it grows,
//!   shows a gp_run call citing another step, a gate run from its shell, a
//!   `git push`, or an edit of the person's games checkout by hand (outside
//!   a gate's patch); the runner stops the worker with why in its record.
//!   What a log line can't show for certain is listed in
//!   docs/worker-evals.md, "Read by hand".

use std::path::Path;

use anyhow::Result;
use serde_json::{Value, json};

use gp_mcp::steps::Runner;

/// Whether ledger line `l` is a try that failed: one that ended in
/// anything but done, and wasn't stopped by the person.
fn failed(l: &Value) -> bool {
    l["state"].is_string()
        && l["state"] != json!("done")
        && l["stopped_by"] != json!("person")
        && l["why"] != json!("you stopped it")
}

/// The failed tries of step `id` in ledger `lines`, after its last give-up
/// (a line `{"gave_up": true}` of the step, written by [`give_up`]).
pub(crate) fn failed_tries<'a>(lines: &'a [Value], id: &str) -> Vec<&'a Value> {
    let mine: Vec<&Value> = lines.iter().filter(|l| l["step"] == json!(id)).collect();
    let from = mine
        .iter()
        .rposition(|l| l["gave_up"] == json!(true))
        .map_or(0, |i| i + 1);
    mine[from..].iter().copied().filter(|l| failed(l)).collect()
}

/// The open steps less those whose workers failed `cap` tries (0: none
/// left out), so the runner starts no worker on them.
pub(crate) fn not_given_up(open: Vec<Value>, lines: &[Value], cap: u32) -> Vec<Value> {
    if cap == 0 {
        return open;
    }
    open.into_iter()
        .filter(|s| {
            let id = s["id"].as_str().unwrap_or("");
            failed_tries(lines, id).len() < cap as usize
        })
        .collect()
}

fn row_of(step: &Value) -> Option<(String, String)> {
    step["touches"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|t| t.strip_prefix("row:"))
        .and_then(|r| r.split_once('/'))
        .map(|(g, r)| (g.to_owned(), r.to_owned()))
}

/// The tries' endings in words: "try 1 (run 1): WHY; try 2 ...".
fn endings(tries: &[&Value]) -> String {
    tries
        .iter()
        .map(|l| {
            format!(
                "try {} of run {}: {}",
                l["try"].as_u64().unwrap_or(0),
                l["run"].as_u64().unwrap_or(0),
                l["why"]
                    .as_str()
                    .filter(|w| !w.is_empty())
                    .unwrap_or("no why")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// After worker record `rec` ended: when its step's failed tries reach the
/// home's `fail_tries`, files a trouble row with their endings, parks the
/// step's row in line waiting on it and writes a give-up line in the
/// ledger, so the count starts again once the row is set next. Says the
/// trouble row, or None when nothing was given up.
pub(crate) fn give_up(home: &Path, rec: &mut Value, now: u64) -> Result<Option<String>> {
    let cap = gp_mcp::steps::settings(home).runner.fail_tries;
    let id = rec["step"]["id"].as_str().unwrap_or("").to_owned();
    if cap == 0 || id.is_empty() || !failed(rec) {
        return Ok(None);
    }
    let Some((game, row)) = row_of(&rec["step"]) else {
        return Ok(None);
    };
    let ledger = crate::worker_runs::ledger(home);
    let lines = crate::worker_runs::lines_of(&ledger);
    let tries = failed_tries(&lines, &id);
    if tries.len() < cap as usize {
        return Ok(None);
    }
    let what = format!(
        "The workers on row {row} failed {} tries at step {id}: {}. The runner starts no worker on the row until this row is done or the row is set next again.",
        tries.len(),
        endings(&tries)
    );
    let paths = gp_evidence::Paths::resolve(Some(home.to_path_buf()), None, None)?;
    let from = format!("gp serve's runner, worker loop-{id}");
    let filed = crate::trouble::file(
        &paths,
        &crate::trouble::Trouble {
            game: &game,
            kind: crate::trouble::Kind::Worker,
            what: &what,
            from: &from,
        },
        now,
    )?;
    let trouble = filed["row"].as_str().unwrap_or("").to_owned();
    park_on(home, &game, &row, &trouble)?;
    crate::worker_runs::append(
        &ledger,
        &json!({"at_ms": now, "step": id, "row": format!("{game}/{row}"), "gave_up": true, "trouble": trouble}),
    );
    rec["trouble_row"] = json!(trouble);
    Ok(Some(trouble))
}

/// Parks `row` of `game` in line waiting on trouble row `trouble`: the
/// loop's own wait (crate::mechanic::waits wakes it once the trouble row is
/// done), not one on the person. A done row stays done.
fn park_on(home: &Path, game: &str, row: &str, trouble: &str) -> Result<()> {
    if !crate::mechanic::session::name_ok(game) || trouble.is_empty() {
        return Ok(());
    }
    let f = home.join("mechanics").join(game).join("queue.json");
    crate::mechanic::session::update_queue(&f, |q| {
        let Some(r) = q
            .iter_mut()
            .find(|r| r["mechanic"] == json!(row) && r["status"] != json!("done"))
        else {
            return Ok(());
        };
        let why =
            format!("its workers failed too many tries; trouble row {trouble} has their endings");
        crate::mechanic::park_row(r, &why);
        r["waits_on"] = json!(trouble);
        r["waits_since"] = json!(gp_mcp::asks::now_ms());
        Ok(())
    })
}

/// What the day's runs in ledger `lines` spent, in dollars: every line of
/// the same day as `now_ms` (UTC, as gp loop stats counts days).
pub(crate) fn day_spend(lines: &[Value], now_ms: u64) -> f64 {
    let today = crate::worker_runs::day_of(now_ms);
    lines
        .iter()
        .filter(|l| {
            l["at_ms"]
                .as_u64()
                .is_some_and(|at| crate::worker_runs::day_of(at) == today)
        })
        .filter_map(|l| l["result"]["cost_usd"].as_f64())
        .sum()
}

/// Why the runner starts no worker now, or None: the day's spend is at its
/// cap, or the machine's `load` is over the ceiling (the `cores` when the
/// settings name none).
pub(crate) fn held(
    r: &Runner,
    lines: &[Value],
    load: f64,
    cores: usize,
    now_ms: u64,
) -> Option<String> {
    if r.day_spend_usd > 0.0 {
        let spent = day_spend(lines, now_ms);
        if spent >= r.day_spend_usd {
            return Some(format!(
                "the day's runs spent {spent:.2} dollars, at the day's cap of {} (gp loop set --day-spend): no worker starts until tomorrow (UTC)",
                r.day_spend_usd
            ));
        }
    }
    let ceiling = if r.max_load > 0.0 {
        r.max_load
    } else {
        cores as f64
    };
    (load > ceiling).then(|| {
        format!(
            "the machine's load {load:.1} is over the ceiling of {ceiling} (gp loop set --max-load): no new worker starts until it falls"
        )
    })
}

/// gp loop health's line for a hold's words `why` from [`held`].
pub(crate) fn health_line(why: &str) -> String {
    format!("HELD     the runner starts no worker: {why}")
}

/// [`held`] for `home` now: its settings, its ledger and the machine's load.
pub(crate) fn held_now(home: &Path) -> Option<String> {
    let r = gp_mcp::steps::settings(home).runner;
    let lines = if r.day_spend_usd > 0.0 {
        crate::worker_runs::lines_of(&crate::worker_runs::ledger(home))
    } else {
        Vec::new()
    };
    let l = crate::worker::Load::now(home);
    held(&r, &lines, l.load, l.cores, gp_mcp::asks::now_ms())
}

/// The gate commands a worker runs only through gp_run, never from its
/// shell.
const GATES: &[&str] = &["model gate", "tool gate", "theme gate"];

/// Why a worker of step `own`, in worktree `wt`, whose stream-json log
/// ends with `text` is stopped, or None: a tool call a log line shows for
/// certain is outside its step (a gp_run call citing another step, a gate
/// run from its shell) or by hand where gp has a tool (a `git push`, an
/// edit or write of a file in the person's games checkout `games`).
pub(crate) fn out_of_step(text: &str, own: &str, games: &Path, wt: &Path) -> Option<String> {
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["type"] != json!("assistant") {
            continue;
        }
        for c in v["message"]["content"].as_array().into_iter().flatten() {
            if c["type"] != json!("tool_use") {
                continue;
            }
            if let Some(why) = call_out_of_step(c, own, games, wt) {
                return Some(format!("{why}, so the runner stopped it"));
            }
        }
    }
    None
}

fn call_out_of_step(c: &Value, own: &str, games: &Path, wt: &Path) -> Option<String> {
    let name = c["name"].as_str().unwrap_or("");
    let input = &c["input"];
    if name.ends_with("gp_run")
        && let Some(step) = input["step"]
            .as_str()
            .filter(|s| !s.is_empty() && *s != own)
    {
        return Some(format!(
            "its gp_run call ({}) cited step {step}, not its own step {own}",
            input["command"].as_str().unwrap_or("?")
        ));
    }
    if name == "Bash" {
        return shell_out_of_step(input["command"].as_str().unwrap_or(""));
    }
    if matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
        let f = input["file_path"]
            .as_str()
            .or_else(|| input["notebook_path"].as_str())?;
        let p = Path::new(f);
        if !games.as_os_str().is_empty() && p.starts_with(games) && !p.starts_with(wt) {
            return Some(format!(
                "it changed {f} in the games checkout by hand ({name}), outside a gate's patch"
            ));
        }
    }
    None
}

/// A shell command's part a worker may not run: a gate from the shell, or
/// a git push.
fn shell_out_of_step(cmd: &str) -> Option<String> {
    for part in cmd.split(['\n', ';', '|', '&']) {
        let words: Vec<&str> = part.split_whitespace().collect();
        let Some(i) = words.iter().position(|w| {
            Path::new(w)
                .file_name()
                .is_some_and(|n| n == "gp" || n == "git")
        }) else {
            continue;
        };
        let prog = Path::new(words[i]).file_name().and_then(|n| n.to_str());
        let rest: Vec<&str> = words[i + 1..]
            .iter()
            .copied()
            .filter(|w| !w.starts_with('-'))
            .collect();
        if prog == Some("git") && rest.contains(&"push") {
            return Some("it ran git push; a change reaches main only through a gate".to_owned());
        }
        if prog == Some("gp") {
            let said = rest.iter().take(2).copied().collect::<Vec<_>>().join(" ");
            if GATES.contains(&said.as_str()) {
                return Some(format!(
                    "it ran gp {said} from its shell; a gate runs only through gp_run, as its step"
                ));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: &str = "s0123456789ab";

    fn step() -> Value {
        json!({"id": STEP, "who": "agent", "journey": "quest log", "touches": ["row:rooftop/jump-height"]})
    }

    fn try_line(n: u64, state: &str, why: &str) -> Value {
        json!({"at_ms": 1_000 + n, "step": STEP, "run": 1, "try": n, "state": state, "why": why, "result": {"cost_usd": 1.0}})
    }

    fn home_with_row() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let d = home.path().join("mechanics").join("rooftop");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("queue.json"),
            serde_json::to_vec(&json!([{"mechanic": "jump-height", "status": "next", "note": "the jump's height"}])).unwrap(),
        )
        .unwrap();
        home
    }

    /// Three failed tries of a step file one trouble row with their
    /// endings, park the step's row waiting on it, and leave the step out
    /// of the runner's starts; a fourth ending files no second row.
    #[test]
    fn three_failed_tries_file_one_trouble_row_and_stop_new_starts() {
        let home = home_with_row();
        let ledger = crate::worker_runs::ledger(home.path());
        for (n, why) in [(1, "exit 1"), (2, "its turn cap")] {
            crate::worker_runs::append(&ledger, &try_line(n, "retrying", why));
        }
        // Two failed tries: the step still starts, and nothing is filed.
        let lines = crate::worker_runs::lines_of(&ledger);
        assert_eq!(not_given_up(vec![step()], &lines, 3).len(), 1);
        let mut rec = json!({"step": step(), "state": "stopped", "why": "exit 1"});
        assert_eq!(give_up(home.path(), &mut rec, 2_000).unwrap(), None);
        // A try the person stopped isn't a failure.
        crate::worker_runs::append(
            &ledger,
            &json!({"step": STEP, "try": 1, "run": 2, "state": "stopped", "stopped_by": "person", "why": "you stopped it"}),
        );
        let lines = crate::worker_runs::lines_of(&ledger);
        assert_eq!(failed_tries(&lines, STEP).len(), 2);
        // The third.
        crate::worker_runs::append(
            &ledger,
            &try_line(3, "no result", "its log has no result line"),
        );
        let lines = crate::worker_runs::lines_of(&ledger);
        assert!(not_given_up(vec![step()], &lines, 3).is_empty());
        let mut rec =
            json!({"step": step(), "state": "no result", "why": "its log has no result line"});
        let trouble = give_up(home.path(), &mut rec, 3_000)
            .unwrap()
            .expect("a trouble row");
        assert_eq!(rec["trouble_row"], json!(trouble));
        let q =
            crate::mechanic::session::read_queue(&home.path().join("mechanics/rooftop/queue.json"));
        let t: Vec<&Value> = q.iter().filter(|r| r["trouble"].is_object()).collect();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0]["mechanic"], json!(trouble));
        assert_eq!(t[0]["trouble"]["kind"], "worker");
        let note = t[0]["note"].as_str().unwrap();
        assert!(
            note.contains("exit 1")
                && note.contains("its turn cap")
                && note.contains("no result line"),
            "{note}"
        );
        let row = q.iter().find(|r| r["mechanic"] == "jump-height").unwrap();
        assert_eq!(row["status"], "in line");
        assert_eq!(row["waits_on"], json!(trouble));
        // The count starts again from the give-up: set next again, the step
        // starts, and one more failure files nothing.
        let lines = crate::worker_runs::lines_of(&ledger);
        assert_eq!(not_given_up(vec![step()], &lines, 3).len(), 1);
        crate::worker_runs::append(&ledger, &try_line(4, "stopped", "exit 2"));
        let mut rec = json!({"step": step(), "state": "stopped", "why": "exit 2"});
        assert_eq!(give_up(home.path(), &mut rec, 4_000).unwrap(), None);
        let q =
            crate::mechanic::session::read_queue(&home.path().join("mechanics/rooftop/queue.json"));
        assert_eq!(q.iter().filter(|r| r["trouble"].is_object()).count(), 1);
        // 0 is no limit.
        assert_eq!(not_given_up(vec![step()], &lines, 0).len(), 1);
    }

    /// A day's ledger over the cap starts no worker and says why; runs of
    /// the day before don't count, and no cap holds nothing.
    #[test]
    fn a_days_ledger_over_the_cap_starts_no_worker() {
        let day = 86_400_000u64;
        let now = 20_000 * day + 3_600_000;
        let lines = vec![
            json!({"at_ms": now - 3_600_000, "result": {"cost_usd": 6.0}}),
            json!({"at_ms": now - 60_000, "result": {"cost_usd": 4.5}}),
            json!({"at_ms": now - day, "result": {"cost_usd": 50.0}}),
            json!({"at_ms": now - 10, "result": null}),
        ];
        assert_eq!(day_spend(&lines, now), 10.5);
        let mut r = Runner {
            day_spend_usd: 10.0,
            ..Runner::default()
        };
        let why = held(&r, &lines, 0.0, 8, now).expect("held");
        assert!(
            why.contains("10.50") && why.contains("--day-spend"),
            "{why}"
        );
        r.day_spend_usd = 11.0;
        assert_eq!(held(&r, &lines, 0.0, 8, now), None);
        r.day_spend_usd = 0.0;
        assert_eq!(held(&r, &lines, 0.0, 8, now), None);
    }

    /// A load over the ceiling starts no worker: the cores by default, the
    /// setting when it names one.
    #[test]
    fn a_load_over_the_ceiling_starts_no_worker() {
        let mut r = Runner::default();
        assert_eq!(held(&r, &[], 7.9, 8, 0), None);
        let why = held(&r, &[], 8.5, 8, 0).expect("held");
        assert!(why.contains("8.5") && why.contains("--max-load"), "{why}");
        r.max_load = 12.0;
        assert_eq!(held(&r, &[], 8.5, 8, 0), None);
        assert!(held(&r, &[], 12.5, 8, 0).is_some());
    }

    fn call(name: &str, input: Value) -> String {
        json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1", "name": name, "input": input}]}}).to_string()
    }

    /// A log with a write outside its step, a gate from the shell, a git
    /// push or an edit of the games checkout stops its worker; its own
    /// step's calls, reads, a commit in its worktree and edits there don't.
    #[test]
    fn a_log_that_steps_outside_its_step_stops_its_worker() {
        let games = Path::new("/home/rooftop/games");
        let wt = Path::new("/home/rooftop/.gp/work/loop/trees/s0123456789ab");
        let fine = [
            call(
                "mcp__gp__gp_run",
                json!({"command": "mechanic log", "step": STEP}),
            ),
            call("mcp__gp__gp_run", json!({"command": "mechanic busy"})),
            call(
                "Bash",
                json!({"command": "/x/bin/gp mechanic show rooftop jump-height"}),
            ),
            call(
                "Bash",
                json!({"command": "git commit -m 'rooftop: a jump'"}),
            ),
            call("Bash", json!({"command": "git diff > wave.patch"})),
            call(
                "Edit",
                json!({"file_path": format!("{}/crates/a.rs", wt.display())}),
            ),
            call(
                "Read",
                json!({"file_path": "/home/rooftop/games/rooftop/model.toml"}),
            ),
            "not json".to_owned(),
        ]
        .join("\n");
        assert_eq!(out_of_step(&fine, STEP, games, wt), None);
        let cases = [
            (
                call(
                    "mcp__gp__gp_run",
                    json!({"command": "tool gate", "step": "s999999999999"}),
                ),
                "cited step s999999999999",
            ),
            (
                call(
                    "Bash",
                    json!({"command": "/x/bin/gp model gate rooftop --patch p"}),
                ),
                "gp model gate from its shell",
            ),
            (
                call(
                    "Bash",
                    json!({"command": "git status && git push origin main"}),
                ),
                "git push",
            ),
            (
                call(
                    "Write",
                    json!({"file_path": "/home/rooftop/games/rooftop/model.toml"}),
                ),
                "games checkout by hand",
            ),
        ];
        for (line, says) in cases {
            let text = format!("{fine}\n{line}");
            let why = out_of_step(&text, STEP, games, wt).unwrap_or_default();
            assert!(
                why.contains(says) && why.ends_with("so the runner stopped it"),
                "{says}: {why}"
            );
        }
    }
}
