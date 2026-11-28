//! What each worker run cost and did (issue 50). A `claude` worker's log
//! (`$GP_HOME/work/loop/files/ID/N.log`, stream-json) ends with a result
//! line: the run's cost, turns, token usage with cache reads and writes,
//! duration, how it ended and the tool calls it was refused. The runner
//! copies those onto the worker's record (`result`) and appends a line per
//! ended try to the ledger, `$GP_HOME/work/loop/runs.jsonl`, since a
//! worker's record and logs are written over by its step's next run.
//!
//! `gp loop stats` reads the ledger, the gates' landed lines (each names
//! the worker or the hand its patch came from, crate::gate_hold::landed)
//! and the cards, and prints for a period: runs, landings, the cost of a
//! landing, the cost of the runs that landed nothing, the cache hit rate,
//! refusals a run and how long a card waited for the person.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The ledger: one JSON line per ended try of a worker.
pub(crate) fn ledger(home: &Path) -> PathBuf {
    gp_mcp::steps::loop_dir(home).join("runs.jsonl")
}

/// The last result line of a worker's stream-json log; None when the log
/// has none (the run was cut off before its end, or the worker's command
/// isn't one that writes stream-json).
pub(crate) fn result_line(log: &Path) -> Option<Value> {
    let b = fs::read(log).ok()?;
    let t = String::from_utf8_lossy(&b);
    t.lines().rev().find_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        (v["type"] == json!("result")).then_some(v)
    })
}

/// What a result line says of its run, under the names the record and the
/// ledger keep: how it ended (`subtype`, `is_error`), `cost_usd`, `turns`,
/// `duration_ms`, the tokens read, written and cached, and how many tool
/// calls were refused (`denials`).
pub(crate) fn summary(result: &Value) -> Value {
    let u = &result["usage"];
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    json!({
        "subtype": result["subtype"].as_str().unwrap_or(""),
        "is_error": result["is_error"].as_bool().unwrap_or(false),
        "cost_usd": result["total_cost_usd"].as_f64().unwrap_or(0.0),
        "turns": n(&result["num_turns"]),
        "duration_ms": n(&result["duration_ms"]),
        "input_tokens": n(&u["input_tokens"]),
        "output_tokens": n(&u["output_tokens"]),
        "cache_read_tokens": n(&u["cache_read_input_tokens"]),
        "cache_write_tokens": n(&u["cache_creation_input_tokens"]),
        "denials": result["permission_denials"].as_array().map_or(0, Vec::len),
        "denials_by": denials_by(result),
    })
}

/// What a refused tool call was, short enough to count by: for Bash the
/// command's first word (`ls`, `nice`), with the second for git (`git -C`,
/// `git apply`); for any other tool its name.
fn denied_command(denial: &Value) -> String {
    let tool = denial["tool_name"].as_str().unwrap_or("?");
    if tool != "Bash" {
        return tool.to_owned();
    }
    let mut words = denial["tool_input"]["command"]
        .as_str()
        .unwrap_or("")
        .split_whitespace();
    match (words.next(), words.next()) {
        (Some("git"), Some(second)) => format!("git {second}"),
        (Some(first), _) => first.to_owned(),
        (None, _) => "Bash".to_owned(),
    }
}

/// The refused tool calls of a result line, counted by command (issue
/// 54): `{"git -C": 4, "ls": 1}`.
pub(crate) fn denials_by(result: &Value) -> BTreeMap<String, usize> {
    let mut by = BTreeMap::new();
    for d in result["permission_denials"]
        .as_array()
        .into_iter()
        .flatten()
    {
        *by.entry(denied_command(d)).or_default() += 1;
    }
    by
}

/// Whether a tool result's words are a refusal by the worker's
/// permissions: the agent's own ("permission ... denied", "haven't
/// granted") and no other error.
fn is_refusal(words: &str) -> bool {
    let w = words.to_lowercase();
    w.contains("permission")
        && (w.contains("denied") || w.contains("haven't granted") || w.contains("not allowed"))
}

/// How many tool calls in a row, ending at the end of a stream-json log's
/// text, were refused by the worker's permissions. A call that ran, well
/// or with an error of its own, ends the run.
pub(crate) fn refusals_in_a_row(text: &str) -> usize {
    let mut n = 0;
    for l in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else {
            continue;
        };
        if v["type"] != json!("user") {
            continue;
        }
        for c in v["message"]["content"].as_array().into_iter().flatten() {
            if c["type"] != json!("tool_result") {
                continue;
            }
            let said = match &c["content"] {
                Value::String(s) => s.clone(),
                Value::Array(a) => a
                    .iter()
                    .filter_map(|x| x["text"].as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            if c["is_error"] == json!(true) && is_refusal(&said) {
                n += 1;
            } else {
                n = 0;
            }
        }
    }
    n
}

/// How a `claude` worker's run ended, read from its log's result line
/// (issue 49). A process that exits 0 is not thereby a run that finished:
/// a run cut off by its turn or spend cap exits like any other, and a log
/// with no result line was cut off before its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ending {
    /// Its result says it ended its work.
    Finished,
    /// A cap cut it off: which, in words.
    CutOff(String),
    /// Its result is an error: the result's subtype.
    Errored(String),
    /// Its log has no result line.
    NoResult,
}

impl Ending {
    /// The state a worker's record ends in when its last try ended so;
    /// None for a run that finished (the runner's own states say the rest).
    pub(crate) fn state(&self) -> Option<&'static str> {
        match self {
            Ending::Finished => None,
            Ending::CutOff(_) => Some("cut off"),
            Ending::Errored(_) => Some("errored"),
            Ending::NoResult => Some("no result"),
        }
    }

    /// How the run ended, for the next try's brief and the record's why.
    pub(crate) fn says(&self) -> Option<String> {
        match self {
            Ending::Finished => None,
            Ending::CutOff(what) => Some(format!("being cut off by {what}")),
            Ending::Errored(sub) => Some(format!("an error ({sub})")),
            Ending::NoResult => Some("no result line in its log".to_owned()),
        }
    }
}

/// How the run whose result line is `result` ended. A subtype naming a
/// cap (`error_max_turns`, `error_max_budget_usd`, any `error_max_...`) is
/// a cut-off; any other error subtype, or `is_error`, an error.
pub(crate) fn ending(result: Option<&Value>) -> Ending {
    let Some(r) = result else {
        return Ending::NoResult;
    };
    let sub = r["subtype"].as_str().unwrap_or("");
    if let Some(cap) = sub.strip_prefix("error_max_") {
        return Ending::CutOff(match cap {
            "turns" => "its turn cap".to_owned(),
            "budget_usd" => "its spend cap".to_owned(),
            other => format!("a cap ({other})"),
        });
    }
    if sub.starts_with("error") || r["is_error"] == json!(true) {
        return Ending::Errored(if sub.is_empty() { "error" } else { sub }.to_owned());
    }
    Ending::Finished
}

/// The tool calls of a stream-json log's text, in order: each call's name
/// and its arguments as written.
fn tool_calls(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for l in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else {
            continue;
        };
        if v["type"] != json!("assistant") {
            continue;
        }
        for c in v["message"]["content"].as_array().into_iter().flatten() {
            if c["type"] == json!("tool_use") {
                out.push((
                    c["name"].as_str().unwrap_or("").to_owned(),
                    c["input"].to_string(),
                ));
            }
        }
    }
    out
}

/// The newest tool call of a log's text and how many times in a row it
/// was made with the same arguments, ending at the log's end; None when
/// the text has no tool call.
pub(crate) fn repeats(text: &str) -> Option<(String, usize)> {
    let calls = tool_calls(text);
    let last = calls.last()?;
    let n = calls.iter().rev().take_while(|c| *c == last).count();
    Some((last.0.clone(), n))
}

/// The end of a log, at most `bytes` of it, as text. Its first line may
/// be cut, which no reader of JSON lines takes for a line.
pub(crate) fn tail(log: &Path, bytes: u64) -> String {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let Ok(mut f) = fs::File::open(log) else {
        return String::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if f.seek(SeekFrom::Start(len.saturating_sub(bytes))).is_err() {
        return String::new();
    }
    let mut b = Vec::new();
    let _ = f.read_to_end(&mut b);
    String::from_utf8_lossy(&b).into_owned()
}

/// Notes the try of worker record `rec` whose log is `log`: its result's
/// [`summary`] (null when the log has no result line) goes onto the record
/// as `result` and into the ledger with the step, the row, the try and the
/// state the try ended in.
pub(crate) fn note(home: &Path, rec: &mut Value, log: &Path, now_ms: u64) {
    let result = result_line(log).map(|r| summary(&r)).unwrap_or(Value::Null);
    rec["result"] = result.clone();
    let row = rec["step"]["touches"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find_map(|t| t.strip_prefix("row:"))
        .map(str::to_owned);
    let line = json!({
        "at_ms": now_ms,
        "step": rec["step"]["id"],
        "worker": rec["worker"],
        "row": row,
        "repo": rec["step"]["repo"],
        "run": rec["runs"],
        "try": rec["tries"],
        "state": rec["state"],
        "why": rec["why"],
        "stopped_by": rec["stopped_by"],
        "log": log,
        "result": result,
    });
    append(&ledger(home), &line);
}

pub(crate) fn append(file: &Path, line: &Value) {
    use std::io::Write as _;
    if let Some(d) = file.parent() {
        let _ = fs::create_dir_all(d);
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

/// A file's JSON lines, those that read.
pub(crate) fn lines_of(file: &Path) -> Vec<Value> {
    let b = fs::read(file).unwrap_or_default();
    String::from_utf8_lossy(&b)
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// The gates' landed lines (crate::gate_hold::landed) in steps files
/// `files`: each `{at_ms, landed, from}`.
pub(crate) fn landings(files: &[PathBuf]) -> Vec<Value> {
    files
        .iter()
        .flat_map(|f| lines_of(f))
        .filter(|l| l["landed"].is_string())
        .collect()
}

fn median(mut v: Vec<u64>) -> Option<u64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

/// The numbers of `runs` (ledger lines), `landings` (landed lines) and
/// `asks` (cards), all taken as given: the caller picks the period.
///
/// A landing is a worker's when one of its `from` names a worker
/// (`worker loop-ID`), else a hand's. The cost of a landing is every
/// worker run's cost over the workers' landings, so the runs that landed
/// nothing are paid for by the ones that did; `cost_landed_nothing_usd` is
/// the cost of the runs whose worker has no landing among `landings`. The
/// cache hit rate is the tokens read from the cache over all tokens read
/// (cached, written to the cache, and plain input). A card's wait is from
/// when it was made to the person's answer; cards the gate answered itself
/// are left out.
pub(crate) fn stats(runs: &[Value], landings: &[Value], asks: &[Value]) -> Value {
    let from_workers = |l: &Value| -> Vec<String> {
        l["from"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|f| f.strip_prefix("worker "))
            .map(str::to_owned)
            .collect()
    };
    let landed_workers: BTreeSet<String> = landings.iter().flat_map(from_workers).collect();
    let by_workers = landings
        .iter()
        .filter(|l| !from_workers(l).is_empty())
        .count();
    let with_result: Vec<&Value> = runs.iter().filter(|r| r["result"].is_object()).collect();
    let f = |r: &Value, k: &str| r["result"][k].as_f64().unwrap_or(0.0);
    let cost: f64 = with_result.iter().map(|r| f(r, "cost_usd")).sum();
    let cost_nothing: f64 = with_result
        .iter()
        .filter(|r| {
            !r["worker"]
                .as_str()
                .is_some_and(|w| landed_workers.contains(w))
        })
        .map(|r| f(r, "cost_usd"))
        .sum();
    let sum = |k: &str| -> f64 { with_result.iter().map(|r| f(r, k)).sum() };
    let (read, written, plain) = (
        sum("cache_read_tokens"),
        sum("cache_write_tokens"),
        sum("input_tokens"),
    );
    let all_in = read + written + plain;
    let steps: BTreeSet<&str> = runs.iter().filter_map(|r| r["worker"].as_str()).collect();
    let steps_landed = steps
        .iter()
        .filter(|w| landed_workers.contains(**w))
        .count();
    let waits: Vec<u64> = asks
        .iter()
        .filter(|a| a["answered_by"] != json!("the gate"))
        .filter_map(|a| {
            let (made, answered) = (a["made_ms"].as_u64()?, a["answered_ms"].as_u64()?);
            Some(answered.saturating_sub(made))
        })
        .collect();
    let worked: Vec<u64> = with_result
        .iter()
        .map(|r| f(r, "duration_ms") as u64)
        .collect();
    let per = |a: f64, b: usize| (b > 0).then(|| a / b as f64);
    let round = |v: Option<f64>, to: f64| v.map(|x| (x * to).round() / to);
    json!({
        "runs": runs.len(),
        "runs_with_result": with_result.len(),
        "steps": steps.len(),
        "steps_landed": steps_landed,
        "landings": landings.len(),
        "landings_by_workers": by_workers,
        "landings_by_hand": landings.len() - by_workers,
        "cost_usd": (cost * 100.0).round() / 100.0,
        "cost_per_landing_usd": round(per(cost, by_workers), 100.0),
        "cost_landed_nothing_usd": (cost_nothing * 100.0).round() / 100.0,
        "cache_hit": round((all_in > 0.0).then(|| read / all_in), 1000.0),
        "denials": sum("denials"),
        "denials_per_run": round(per(sum("denials"), with_result.len()), 100.0),
        "turns_per_run": round(per(sum("turns"), with_result.len()), 10.0),
        "card_wait_median_ms": median(waits),
        "worked_median_ms": median(worked),
    })
}

fn minutes(ms: &Value) -> String {
    match ms.as_u64() {
        Some(m) => format!("{:.1} min", m as f64 / 60_000.0),
        None => "none".to_owned(),
    }
}

fn dollars(v: &Value) -> String {
    match v.as_f64() {
        Some(d) => format!("${d:.2}"),
        None => "none".to_owned(),
    }
}

/// [`stats`] as lines a person reads.
pub(crate) fn words(s: &Value) -> Vec<String> {
    let n = |k: &str| s[k].as_u64().unwrap_or(0);
    let share = |k: &str| match s[k].as_f64() {
        Some(v) => format!("{:.1}%", v * 100.0),
        None => "none".to_owned(),
    };
    let num = |k: &str| match s[k].as_f64() {
        Some(v) => format!("{v}"),
        None => "none".to_owned(),
    };
    vec![
        format!(
            "runs: {} ({} with a result line) on {} steps, {} of them with a landing",
            n("runs"),
            n("runs_with_result"),
            n("steps"),
            n("steps_landed")
        ),
        format!(
            "landings: {} ({} by workers, {} by hand)",
            n("landings"),
            n("landings_by_workers"),
            n("landings_by_hand")
        ),
        format!(
            "cost: {} in all, {} a worker's landing, {} on runs that landed nothing",
            dollars(&s["cost_usd"]),
            dollars(&s["cost_per_landing_usd"]),
            dollars(&s["cost_landed_nothing_usd"])
        ),
        format!("cache hit rate: {}", share("cache_hit")),
        format!(
            "refused tool calls: {} a run; turns: {} a run",
            num("denials_per_run"),
            num("turns_per_run")
        ),
        format!(
            "median wait of a card for the person: {}; median time a worker worked: {}",
            minutes(&s["card_wait_median_ms"]),
            minutes(&s["worked_median_ms"])
        ),
    ]
}

/// The day (`YYYY-MM-DD`, UTC) of a time in milliseconds.
pub(crate) fn day_of(ms: u64) -> String {
    crate::game_new::date_of_days((ms / 86_400_000) as i64)
}

/// `gp loop stats`: the numbers of the last `days` days up to `now_ms`,
/// in all and for each day that has a run or a landing.
pub(crate) fn report(
    runs: &[Value],
    landings: &[Value],
    asks: &[Value],
    days: u64,
    now_ms: u64,
) -> Value {
    let from = now_ms.saturating_sub(days * 86_400_000);
    let within = |v: &&Value, k: &str| v[k].as_u64().is_some_and(|t| t >= from && t <= now_ms);
    let runs: Vec<Value> = runs
        .iter()
        .filter(|r| within(r, "at_ms"))
        .cloned()
        .collect();
    let landings: Vec<Value> = landings
        .iter()
        .filter(|l| within(l, "at_ms"))
        .cloned()
        .collect();
    let asks: Vec<Value> = asks
        .iter()
        .filter(|a| within(a, "made_ms"))
        .cloned()
        .collect();
    // A day's runs, landings and cards.
    type Day = (Vec<Value>, Vec<Value>, Vec<Value>);
    let mut by_day: BTreeMap<String, Day> = BTreeMap::new();
    for r in &runs {
        let d = day_of(r["at_ms"].as_u64().unwrap_or(0));
        by_day.entry(d).or_default().0.push(r.clone());
    }
    for l in &landings {
        let d = day_of(l["at_ms"].as_u64().unwrap_or(0));
        by_day.entry(d).or_default().1.push(l.clone());
    }
    for a in &asks {
        let d = day_of(a["made_ms"].as_u64().unwrap_or(0));
        if let Some(e) = by_day.get_mut(&d) {
            e.2.push(a.clone());
        }
    }
    let per_day: Vec<Value> = by_day
        .iter()
        .map(|(d, (r, l, a))| {
            let mut s = stats(r, l, a);
            s["day"] = json!(d);
            s
        })
        .collect();
    let all = stats(&runs, &landings, &asks);
    let mut lines = vec![format!("The loop's workers over the last {days} day(s):")];
    lines.extend(words(&all).into_iter().map(|l| format!("  {l}")));
    for d in &per_day {
        lines.push(format!(
            "  {}: {} runs, {} landings ({} by workers), {} in all, {} a worker's landing, cache {}",
            d["day"].as_str().unwrap_or("?"),
            d["runs"],
            d["landings"],
            d["landings_by_workers"],
            dollars(&d["cost_usd"]),
            dollars(&d["cost_per_landing_usd"]),
            match d["cache_hit"].as_f64() {
                Some(v) => format!("{:.1}%", v * 100.0),
                None => "none".to_owned(),
            }
        ));
    }
    if runs.is_empty() {
        lines.push(
            "  No run is in the ledger for the period: it is written from this gp on, as each worker's try ends."
                .to_owned(),
        );
    }
    json!({"days": days, "all": all, "per_day": per_day, "lines": lines})
}

/// The metrics `gp loop stats` records for a day's numbers (the run
/// recorder's store, `$GP_HOME/metrics.sqlite`), so a change to the brief,
/// the tools or the model can be compared before and after: name, value
/// and unit, for each number the day has.
pub(crate) fn metrics_of(day: &Value) -> Vec<(String, f64, &'static str)> {
    let mut out = Vec::new();
    for (key, unit) in [
        ("runs", "count"),
        ("landings", "count"),
        ("landings_by_workers", "count"),
        ("cost_usd", "usd"),
        ("cost_per_landing_usd", "usd"),
        ("cost_landed_nothing_usd", "usd"),
        ("cache_hit", "ratio"),
        ("denials_per_run", "count"),
        ("turns_per_run", "count"),
        ("card_wait_median_ms", "ms"),
        ("worked_median_ms", "ms"),
    ] {
        if let Some(v) = day[key].as_f64() {
            out.push((format!("loop.workers.{key}"), v, unit));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":120000,"num_turns":12,"total_cost_usd":1.5,"usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":800,"cache_creation_input_tokens":100},"permission_denials":[{"tool_name":"Bash"},{"tool_name":"Bash"}]}"#;

    /// Issue 50: the result line a worker's log ends with is read into the
    /// record and the ledger.
    #[test]
    fn a_runs_result_line_goes_onto_its_record_and_into_the_ledger() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let log = t.path().join("1.log");
        fs::write(
            &log,
            format!("{{\"type\":\"assistant\"}}\n{RESULT}\nnot json\n"),
        )
        .unwrap();
        let mut rec = json!({
            "step": {"id": "s0000000000e1", "touches": ["row:g/r"], "repo": "tools"},
            "worker": "loop-s0000000000e1", "state": "done", "tries": 1, "runs": 2,
        });
        note(&home, &mut rec, &log, 5);
        let r = &rec["result"];
        assert_eq!(r["cost_usd"], 1.5);
        assert_eq!(r["turns"], 12);
        assert_eq!(r["duration_ms"], 120_000);
        assert_eq!(r["input_tokens"], 100);
        assert_eq!(r["output_tokens"], 50);
        assert_eq!(r["cache_read_tokens"], 800);
        assert_eq!(r["cache_write_tokens"], 100);
        assert_eq!(r["denials"], 2);
        assert_eq!(r["subtype"], "success");
        let l = lines_of(&ledger(&home));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0]["worker"], "loop-s0000000000e1");
        assert_eq!(l[0]["row"], "g/r");
        assert_eq!(l[0]["at_ms"], 5);
        assert_eq!(l[0]["result"]["cost_usd"], 1.5);
        // A log with no result line: the try is in the ledger, with none.
        let bare = t.path().join("2.log");
        fs::write(&bare, "hello\n").unwrap();
        note(&home, &mut rec, &bare, 6);
        assert!(rec["result"].is_null());
        assert_eq!(lines_of(&ledger(&home)).len(), 2);
    }

    /// Issue 49: how a run ended is read from its result line.
    #[test]
    fn a_result_line_says_how_its_run_ended() {
        let r = |s: &str, e: bool| json!({"type": "result", "subtype": s, "is_error": e});
        assert_eq!(ending(Some(&r("success", false))), Ending::Finished);
        assert_eq!(
            ending(Some(&r("error_max_turns", false))),
            Ending::CutOff("its turn cap".into())
        );
        assert_eq!(
            ending(Some(&r("error_max_budget_usd", true))),
            Ending::CutOff("its spend cap".into())
        );
        assert_eq!(
            ending(Some(&r("error_during_execution", true))),
            Ending::Errored("error_during_execution".into())
        );
        assert_eq!(
            ending(Some(&r("success", true))),
            Ending::Errored("success".into())
        );
        assert_eq!(ending(None), Ending::NoResult);
        let states: Vec<Option<&str>> = [
            Ending::Finished,
            Ending::CutOff(String::new()),
            Ending::Errored(String::new()),
            Ending::NoResult,
        ]
        .iter()
        .map(Ending::state)
        .collect();
        assert_eq!(
            states,
            [None, Some("cut off"), Some("errored"), Some("no result")]
        );
    }

    /// Issue 54: a run's refusals are counted by command on its record, and
    /// refusals in a row at a log's end are counted for the runner's stop.
    #[test]
    fn refusals_are_counted_by_command_and_in_a_row() {
        let bash = |c: &str| json!({"tool_name": "Bash", "tool_input": {"command": c}});
        let result = json!({"type": "result", "permission_denials": [
            bash("git -C /x log --oneline"),
            bash("git -C /y status"),
            bash("nice -n 10 cargo build"),
            bash("ls -la"),
            bash("git apply --check p.patch"),
            {"tool_name": "WebFetch", "tool_input": {"url": "https://example.com"}},
        ]});
        let by = denials_by(&result);
        assert_eq!(by["git -C"], 2);
        assert_eq!(by["nice"], 1);
        assert_eq!(by["ls"], 1);
        assert_eq!(by["git apply"], 1);
        assert_eq!(by["WebFetch"], 1);
        let s = summary(&result);
        assert_eq!(s["denials"], 6);
        assert_eq!(s["denials_by"]["git -C"], 2);
        let out = |words: &str, err: bool| {
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "is_error": err, "content": words},
            ]}})
            .to_string()
        };
        let refused = out(
            "Permission to use Bash with command ls has been denied.",
            true,
        );
        let asked = json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "is_error": true, "content": [
                {"type": "text", "text": "Claude requested permissions to use Bash, but you haven't granted it yet."},
            ]},
        ]}})
        .to_string();
        let ran = out("a.txt", false);
        let failed = out("error: no such file", true);
        assert_eq!(refusals_in_a_row(""), 0);
        assert_eq!(
            refusals_in_a_row(&[&refused, &asked, &refused].map(String::as_str).join("\n")),
            3
        );
        assert_eq!(
            refusals_in_a_row(&format!("{refused}\n{refused}\n{ran}\n{refused}\n")),
            1,
            "a call that ran ends the run"
        );
        assert_eq!(
            refusals_in_a_row(&format!("{refused}\n{failed}\n")),
            0,
            "an error of the call's own is no refusal"
        );
    }

    /// The end of a log is read without the rest, and a call repeated at
    /// its end is counted.
    #[test]
    fn the_tail_of_a_log_gives_its_newest_calls() {
        let t = tempfile::tempdir().unwrap();
        let log = t.path().join("1.log");
        let call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/a"}}]}}"#;
        fs::write(&log, format!("{call}\n{call}\n{call}\n")).unwrap();
        let all = tail(&log, 1 << 20);
        assert_eq!(repeats(&all), Some(("Read".into(), 3)));
        // A tail that cuts the first line counts the whole lines after it.
        let cut = tail(&log, (call.len() * 2 + 10) as u64);
        assert_eq!(repeats(&cut), Some(("Read".into(), 2)));
        assert_eq!(tail(&t.path().join("none.log"), 100), "");
    }

    fn run(worker: &str, cost: f64, at: u64) -> Value {
        json!({"at_ms": at, "worker": worker, "result": {
            "cost_usd": cost, "turns": 10, "duration_ms": 60_000, "denials": 3,
            "input_tokens": 10, "cache_read_tokens": 80, "cache_write_tokens": 10,
        }})
    }

    /// Issue 50: cost per landing, the cost of runs that landed nothing,
    /// the cache hit rate, refusals a run and the card's wait.
    #[test]
    fn the_stats_count_cost_per_landing_and_what_landed_nothing() {
        let runs = [
            run("loop-a", 2.0, 10),
            run("loop-a", 1.0, 20),
            run("loop-b", 3.0, 30),
            json!({"at_ms": 40, "worker": "loop-c", "result": null}),
        ];
        let landings = [
            json!({"at_ms": 25, "landed": "abc", "from": ["worker loop-a"]}),
            json!({"at_ms": 26, "landed": "def", "from": ["hand"]}),
        ];
        let asks = [
            json!({"made_ms": 0, "answered_ms": 60_000}),
            json!({"made_ms": 0, "answered_ms": 180_000}),
            json!({"made_ms": 0, "answered_ms": 300_000}),
            json!({"made_ms": 0, "answered_ms": 1, "answered_by": "the gate"}),
            json!({"made_ms": 0}),
        ];
        let s = stats(&runs, &landings, &asks);
        assert_eq!(s["runs"], 4);
        assert_eq!(s["runs_with_result"], 3);
        assert_eq!(s["steps"], 3);
        assert_eq!(s["steps_landed"], 1);
        assert_eq!(s["landings"], 2);
        assert_eq!(s["landings_by_workers"], 1);
        assert_eq!(s["landings_by_hand"], 1);
        assert_eq!(s["cost_usd"], 6.0);
        assert_eq!(s["cost_per_landing_usd"], 6.0);
        assert_eq!(s["cost_landed_nothing_usd"], 3.0);
        assert_eq!(s["cache_hit"], 0.8);
        assert_eq!(s["denials_per_run"], 3.0);
        assert_eq!(s["card_wait_median_ms"], 180_000);
        assert_eq!(s["worked_median_ms"], 60_000);
        let w = words(&s).join("\n");
        assert!(w.contains("$6.00 a worker's landing"), "{w}");
        assert!(w.contains("cache hit rate: 80.0%"), "{w}");
        // No landing by a worker: no cost of one, and nothing divides by 0.
        let s = stats(&runs, &[], &[]);
        assert!(s["cost_per_landing_usd"].is_null());
        assert_eq!(s["cost_landed_nothing_usd"], 6.0);
        assert!(s["card_wait_median_ms"].is_null());
        let s = stats(&[], &[], &[]);
        assert!(s["cache_hit"].is_null());
    }

    /// The period holds what is in it, and each day has its own numbers
    /// for the metrics store.
    #[test]
    fn the_report_is_for_the_period_with_a_line_per_day() {
        let day = 86_400_000;
        let now = 10 * day;
        let runs = [
            run("loop-a", 1.0, now - 8 * day),
            run("loop-a", 2.0, now - day - 5),
            run("loop-b", 4.0, now - 5),
        ];
        let landings = [json!({"at_ms": now - 4, "landed": "abc", "from": ["worker loop-b"]})];
        let r = report(&runs, &landings, &[], 7, now);
        assert_eq!(r["all"]["runs"], 2, "the run of 8 days ago is left out");
        assert_eq!(r["all"]["cost_usd"], 6.0);
        let days = r["per_day"].as_array().unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[0]["day"], day_of(now - day - 5));
        assert_eq!(days[1]["landings_by_workers"], 1);
        assert_eq!(days[1]["cost_per_landing_usd"], 4.0);
        let m = metrics_of(&days[1]);
        assert!(
            m.contains(&("loop.workers.cost_per_landing_usd".to_owned(), 4.0, "usd")),
            "{m:?}"
        );
        // A day with no landing by a worker records no cost of one.
        assert!(
            !metrics_of(&days[0])
                .iter()
                .any(|(n, _, _)| n == "loop.workers.cost_per_landing_usd")
        );
    }
}
