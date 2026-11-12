//! The host on the made-up game of `gp_mcp::testgame`. No `gp` binary and no
//! game data; the build job is checked only as far as its failure.

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gp_mcp::testgame;
use serde_json::{Value, json};

use crate::local::{HostConfig, LocalHost};
use crate::scenario::{self, ScenarioFile};
use crate::{Host, JobSpec, JobState, StepRequest, http};

struct Fixture {
    _dir: tempfile::TempDir,
    host: Arc<LocalHost>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (config, _) = testgame::config(dir.path());
        let host = LocalHost::start(HostConfig::new(config)).unwrap();
        Self { _dir: dir, host }
    }

    fn copy(&self) -> std::path::PathBuf {
        self.host.config().mcp.work.join("copy")
    }

    fn api(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let r = http::handle(&self.host, method, path, body.to_string().as_bytes());
        (
            r.status,
            serde_json::from_slice(&r.body).unwrap_or(Value::Null),
        )
    }
}

const SCENARIOS: &str = r#"
format = "0.1.0"

[scenarios.stands-still]
description = "No input: the hero stays on the ground"
frames = 30

[[scenarios.stands-still.expect]]
path = "entities.hero[0].x"
op = "eq"
value = "16"

[scenarios.walks-right]
frames = 40
holds = ["right@0-"]

[[scenarios.walks-right.expect]]
path = "entities.hero[0].x"
op = "gt"
value = "16"

[[scenarios.walks-right.expect]]
at = 0
path = "entities.hero[0].x"
op = "between"
min = "10"
max = "20"

[scenarios.fails-on-purpose]
frames = 5

[[scenarios.fails-on-purpose.expect]]
path = "entities.hero[0].x"
op = "gt"
value = "1000"

[[scenarios.fails-on-purpose.expect]]
event = "stomp"
count_min = 1
"#;

#[test]
fn a_scenario_file_is_checked_when_read() {
    assert!(ScenarioFile::parse(SCENARIOS).is_ok());
    let bad = [
        ("format = \"9.9.9\"", "format"),
        (
            "format = \"0.1.0\"\n[scenarios.a]\nframes = 1\n",
            "expects nothing",
        ),
        (
            "format = \"0.1.0\"\n[scenarios.a]\n[[scenarios.a.expect]]\nstate_hash = \"x\"\n",
            "frames",
        ),
        (
            "format = \"0.1.0\"\n[scenarios.a]\nframes = 1\n[[scenarios.a.expect]]\npath = \"a\"\n",
            "op",
        ),
        (
            "format = \"0.1.0\"\n[scenarios.a]\nframes = 1\n[[scenarios.a.expect]]\npath = \"a\"\nop = \"between\"\nmin = \"1\"\n",
            "max",
        ),
        (
            "format = \"0.1.0\"\n[scenarios.a]\nframes = 1\n[[scenarios.a.expect]]\nevent = \"x\"\n",
            "count",
        ),
        (
            "format = \"0.1.0\"\n[scenarios.a]\nframes = 1\nbaseline = \"b\"\n[[scenarios.a.expect]]\nstate_hash = \"x\"\n",
            "not both",
        ),
    ];
    for (text, want) in bad {
        let e = ScenarioFile::parse(text).unwrap_err();
        assert!(e.contains(want), "{text} -> {e}");
    }
}

#[test]
fn scenarios_run_in_parallel_and_give_the_same_results() {
    let f = Fixture::new();
    fs::write(f.copy().join("scenarios.toml"), SCENARIOS).unwrap();
    let run = |jobs: usize| {
        f.host
            .run_scenarios(&scenario::Request {
                copy: "copy".into(),
                jobs: Some(jobs),
                ..Default::default()
            })
            .unwrap()
    };
    let one = run(1);
    let three = run(3);
    assert_eq!(one["scenarios"], 3);
    assert_eq!(one["passed"], 2, "{one}");
    assert_eq!(one["failed"], 1);
    assert_eq!(one["ok"], false);
    assert_eq!(three["jobs"], 3);
    assert_eq!(
        one["results"], three["results"],
        "threads don't change results"
    );
    assert_eq!(
        one["store_object"], three["store_object"],
        "the object is content-addressed"
    );
    let failing = one["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "fails-on-purpose")
        .unwrap();
    assert_eq!(failing["checks"].as_array().unwrap().len(), 2);
    assert!(
        failing["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["ok"] == false)
    );
    // The result is in the store, by hash.
    let store = gp_evidence::Store::open(f.host.config().mcp.store()).unwrap();
    let hash: gp_evidence::Hash = one["store_object"].as_str().unwrap().parse().unwrap();
    let stored: Value = serde_json::from_slice(&store.get(&hash).unwrap()).unwrap();
    assert_eq!(stored["results"].as_array().unwrap().len(), 3);
    assert!(
        stored.get("millis").is_none(),
        "timings aren't in the object"
    );

    // A name that isn't there is an error that lists the ones that are.
    let e = f
        .host
        .run_scenarios(&scenario::Request {
            copy: "copy".into(),
            names: vec!["nope".into()],
            ..Default::default()
        })
        .unwrap_err();
    assert!(e.contains("walks-right"), "{e}");
    // Only the ones asked for.
    let r = f
        .host
        .run_scenarios(&scenario::Request {
            copy: "copy".into(),
            names: vec!["walks-right".into()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r["ok"], true, "{r}");
}

#[test]
fn a_recorded_scenario_holds_a_variant_to_its_own_play() {
    let f = Fixture::new();
    let rec = scenario::Record {
        copy: "copy".into(),
        name: "hero-walks".into(),
        description: Some("recorded".into()),
        frames: Some(60),
        holds: vec!["right@0-".into()],
        paths: vec!["entities.hero[0].x".into()],
        ..Default::default()
    };
    let r = f.host.record_scenario(&rec).unwrap();
    assert_eq!(r["frames"], 60);
    let again = f.host.record_scenario(&rec).unwrap_err();
    assert!(again.contains("already"), "{again}");
    let run = || {
        f.host
            .run_scenarios(&scenario::Request {
                copy: "copy".into(),
                ..Default::default()
            })
            .unwrap()
    };
    assert_eq!(run()["ok"], true, "{}", run());
    // Another scenario appends without disturbing the first.
    f.host
        .record_scenario(&scenario::Record {
            name: "hero-stands".into(),
            holds: vec![],
            ..rec.clone()
        })
        .unwrap();
    assert_eq!(run()["scenarios"], 2);

    // Edit the model: the walk changes, and the recording says so.
    let text = fs::read_to_string(f.copy().join("model.toml")).unwrap();
    assert!(text.contains("walk_speed = 1"));
    fs::write(
        f.copy().join("model.toml"),
        text.replace("walk_speed = 1", "walk_speed = 2"),
    )
    .unwrap();
    let r = run();
    assert_eq!(r["ok"], false, "{r}");
    let walks = r["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "hero-walks")
        .unwrap();
    assert_eq!(walks["ok"], false);
}

#[test]
fn a_session_follows_edits_to_its_copy() {
    let f = Fixture::new();
    let s = f.host.open_session("copy").unwrap();
    let id = s["session"].as_str().unwrap().to_owned();
    assert_eq!(s["frame"], 0);
    let step = StepRequest {
        frames: Some(30),
        buttons: vec!["right".into()],
        paths: vec!["entities.hero[0].x".into()],
        ..Default::default()
    };
    let a = f.host.step_session(&id, &step).unwrap();
    assert_eq!(a["frame"], 30);
    assert_eq!(a["reloaded"], false);
    let x1: f64 = a["state"]["entities.hero[0].x"].as_str().map_or_else(
        || a["state"]["entities.hero[0].x"].as_f64().unwrap(),
        |s| s.parse().unwrap(),
    );
    assert!(x1 > 16.0);
    let q = f
        .host
        .query_session(&id, &["entities.hero[0].x".into()])
        .unwrap();
    assert_eq!(q["frame"], 30);
    // Edit the model; the next step reloads and replays the 30 frames first.
    let text = fs::read_to_string(f.copy().join("model.toml")).unwrap();
    fs::write(
        f.copy().join("model.toml"),
        text.replace("walk_speed = 1", "walk_speed = 2"),
    )
    .unwrap();
    let b = f.host.step_session(&id, &step).unwrap();
    assert_eq!(b["reloaded"], true);
    assert_eq!(b["frame"], 60);
    assert_eq!(f.host.sessions().len(), 1);
    f.host.close_session(&id).unwrap();
    assert!(f.host.close_session(&id).is_err());
    assert!(f.host.open_session("nope").is_err());
}

fn wait(f: &Fixture, id: &str) -> crate::JobView {
    let t = Instant::now();
    loop {
        let j = f.host.job(id).unwrap();
        if matches!(j.state, JobState::Done | JobState::Failed) {
            return j;
        }
        assert!(
            t.elapsed() < Duration::from_secs(60),
            "job {id} didn't finish"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn jobs_queue_run_and_report() {
    let f = Fixture::new();
    fs::write(f.copy().join("scenarios.toml"), SCENARIOS).unwrap();
    let id = f
        .host
        .submit(JobSpec::Scenarios {
            copy: "copy".into(),
            template: false,
            names: vec!["walks-right".into()],
            jobs: Some(2),
        })
        .unwrap();
    let j = wait(&f, &id);
    assert_eq!(j.state, JobState::Done, "{:?}", j.error);
    assert_eq!(j.result.as_ref().unwrap()["ok"], true);
    assert!(
        f.host.jobs().iter().all(|j| j.result.is_none()),
        "a listing holds no results"
    );

    // A build the stand-in setup can't make fails, with the reason.
    let id = f
        .host
        .submit(JobSpec::Build {
            copy: "copy".into(),
            target: None,
            fetch_tools: false,
        })
        .unwrap();
    let j = wait(&f, &id);
    assert_eq!(j.state, JobState::Failed);
    assert!(j.error.is_some());
    // What can never run is refused at once.
    assert!(
        f.host
            .submit(JobSpec::Build {
                copy: "nope".into(),
                target: None,
                fetch_tools: false
            })
            .is_err()
    );
    assert!(
        f.host
            .submit(JobSpec::Build {
                copy: "copy".into(),
                target: Some("ios".into()),
                fetch_tools: false
            })
            .is_err()
    );
}

#[test]
fn a_published_directory_gets_a_link_the_host_serves() {
    let f = Fixture::new();
    let out = f.host.config().dir.join("builds/copy/x");
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("manifest.json"),
        r#"{"copy":"copy","target":"web-preview"}"#,
    )
    .unwrap();
    fs::write(out.join("index.html"), "<p>play</p>").unwrap();
    fs::write(out.join("gp_web.wasm"), [0, 97, 115, 109]).unwrap();
    f.host.set_base("http://127.0.0.1:1");
    let (status, v) = f.api("POST", "/api/links", json!({ "dir": out }));
    assert_eq!(status, 200, "{v}");
    let link = &v["link"];
    assert!(
        link["url"]
            .as_str()
            .unwrap()
            .starts_with("http://127.0.0.1:1/play/")
    );
    let path = link["path"].as_str().unwrap();
    let r = http::handle(&f.host, "GET", path, b"");
    assert_eq!(
        (r.status, r.content_type),
        (200, "text/html; charset=utf-8")
    );
    assert_eq!(r.body, b"<p>play</p>");
    let r = http::handle(&f.host, "GET", &format!("{path}gp_web.wasm"), b"");
    assert_eq!(r.content_type, "application/wasm");
    assert_eq!(
        http::handle(&f.host, "GET", path.trim_end_matches('/'), b"").status,
        301
    );
    assert_eq!(
        http::handle(&f.host, "GET", &format!("{path}../manifest.json"), b"").status,
        404
    );
    assert_eq!(http::handle(&f.host, "POST", path, b"").status, 405);
    assert_eq!(http::handle(&f.host, "GET", "/play/nope/", b"").status, 404);
    // The links survive a restart.
    let again = LocalHost::start(f.host.config().clone()).unwrap();
    assert_eq!(again.links().len(), 1);
    // And a directory that isn't a build gets none.
    let (status, _) = f.api("POST", "/api/links", json!({ "dir": f.copy() }));
    assert_eq!(status, 422);
}

#[test]
fn speed_measurements_are_held_to_their_budgets() {
    let f = Fixture::new();
    let (s, v) = f.api(
        "POST",
        "/api/speed",
        json!({ "name": "plan_s", "seconds": 4.5 }),
    );
    assert_eq!((s, v["within_budget"].as_bool()), (200, Some(true)), "{v}");
    let (s, v) = f.api(
        "POST",
        "/api/speed",
        json!({ "name": "plan_s", "seconds": 45.0 }),
    );
    assert_eq!((s, v["within_budget"].as_bool()), (200, Some(false)));
    let (s, _) = f.api(
        "POST",
        "/api/speed",
        json!({ "name": "vibes", "seconds": 1.0 }),
    );
    assert_eq!(s, 422);
    let (_, v) = f.api("GET", "/api/speed", Value::Null);
    assert_eq!(v["metrics"]["plan_s"]["samples"], 2);
    assert_eq!(
        v["metrics"]["plan_s"]["within_budget"], false,
        "the last sample counts"
    );
}

#[test]
fn the_api_refuses_bad_requests() {
    let f = Fixture::new();
    assert_eq!(f.api("GET", "/api/health", Value::Null).0, 200);
    assert_eq!(f.api("POST", "/api/sessions", json!({ "copy": 3 })).0, 400);
    assert_eq!(f.api("GET", "/api/nothing", Value::Null).0, 404);
    assert_eq!(f.api("GET", "/api/jobs/j999", Value::Null).0, 404);
    assert_eq!(f.api("POST", "/api/sessions/s9/step", json!({})).0, 422);
}

#[test]
fn the_search_moves_holds_toward_a_goal_and_says_when_it_cannot() {
    let f = Fixture::new();
    let req = |goal: &str| crate::search::Search {
        copy: "copy".into(),
        holds: vec!["right@10-".into()],
        frames: Some(40),
        goal_path: "rules.game.state".into(),
        goal_value: goal.into(),
        progress_path: "entities.hero[0].x".into(),
        ..Default::default()
    };
    // Met from the start; the search still pushes the progress, by pressing
    // right sooner.
    let r = f.host.search_inputs(&req("playing")).unwrap();
    assert_eq!(r["goal_met"], true, "{r}");
    assert_eq!(r["holds"], json!(["right@0-"]));
    assert!(r["progress"].as_f64().unwrap() > r["start"]["progress"].as_f64().unwrap());
    // A state the tiny game doesn't have.
    let r = f.host.search_inputs(&req("won")).unwrap();
    assert_eq!(r["goal_met"], false);
    assert!(r["note"].as_str().unwrap().contains("no holds"), "{r}");
    // Bad input is an error.
    let mut bad = req("playing");
    bad.holds = vec!["right".into()];
    assert!(f.host.search_inputs(&bad).is_err());
    let mut bad = req("playing");
    bad.frames = None;
    assert!(f.host.search_inputs(&bad).is_err());
}

#[test]
fn a_path_pattern_is_read_like_a_path() {
    let f = Fixture::new();
    fs::write(
        f.copy().join("scenarios.toml"),
        r#"
format = "0.1.0"

[scenarios.all-grounded]
frames = 10

[[scenarios.all-grounded.expect]]
path = "entities.hero[*].alive"
op = "eq"
value = "true"

[scenarios.none-flying]
frames = 10

[[scenarios.none-flying.expect]]
path = "entities.hero[*].x"
op = "gt"
value = "1000"
"#,
    )
    .unwrap();
    let r = f
        .host
        .run_scenarios(&scenario::Request {
            copy: "copy".into(),
            ..Default::default()
        })
        .unwrap();
    let by = |n: &str| {
        r["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == n)
            .unwrap()
            .clone()
    };
    assert_eq!(by("none-flying")["ok"], false, "{r}");
    assert!(by("all-grounded")["error"].is_null(), "{r}");
    assert_eq!(by("all-grounded")["ok"], true, "{r}");
}

/// A capture job on a stand-in `gp` that logs its arguments and answers as
/// `gp capture` would: it starts the run, then watches it, on a thread of
/// its own while a build worker stays free.
#[cfg(unix)]
#[test]
fn a_capture_job_starts_and_watches_its_run() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (mut config, _) = testgame::config(dir.path());
    let calls = dir.path().join("calls.log");
    let fake = dir.path().join("gp");
    fs::write(
        &fake,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\ncase \"$3\" in\n  start) mkdir -p \"$GP_HOME/capture/demo\"; echo '{{}}' > \"$GP_HOME/capture/demo/run.json\"; echo '{{\"round\":1,\"open\":1}}' ;;\n  watch) sleep 0.3; echo '{{\"state\":\"steady\",\"rounds\":2}}' ;;\nesac\n",
            calls.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    config.gp = fake;
    config.home = dir.path().join("home");
    let host = LocalHost::start(HostConfig::new(config)).unwrap();
    let f = Fixture {
        _dir: tempfile::tempdir().unwrap(),
        host,
    };
    let (s, v) = f.api(
        "POST",
        "/api/jobs",
        json!({"kind": "capture", "run": "demo"}),
    );
    assert_eq!(s, 422, "a run that doesn't exist needs start: {v}");
    let (s, _) = f.api(
        "POST",
        "/api/jobs",
        json!({"kind": "capture", "run": "../x", "start": {"model": "mosshop"}}),
    );
    assert_eq!(s, 422);
    let (s, v) = f.api(
        "POST",
        "/api/jobs",
        json!({"kind": "capture", "run": "demo", "debounce_ms": 200, "start": {"model": "mosshop", "rom": "/r/m.nes", "baselines": "baselines/mosshop", "preview": true}}),
    );
    assert_eq!(s, 200, "{v}");
    let id = v["job"]["id"].as_str().unwrap().to_owned();
    assert_eq!(v["job"]["kind"], "capture");
    assert_eq!(v["job"]["state"], "running", "it doesn't wait for a worker");
    // A build queued meanwhile still gets the worker.
    let other = f
        .host
        .submit(JobSpec::Scenarios {
            copy: "copy".into(),
            template: false,
            names: vec![],
            jobs: Some(1),
        })
        .unwrap();
    let j = wait(&f, &other);
    assert_ne!(j.state, JobState::Queued);
    let j = wait(&f, &id);
    assert_eq!(j.state, JobState::Done, "{:?}", j.error);
    let r = j.result.unwrap();
    assert_eq!(r["watch"]["state"], "steady");
    assert_eq!(r["start"]["open"], 1);
    let log = fs::read_to_string(&calls).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        lines,
        [
            "--json capture start mosshop --rom /r/m.nes --baselines baselines/mosshop --preview --run demo",
            "--json capture watch demo --debounce-ms 200",
        ]
    );
    // Now the run exists: start is refused, a bare watch is taken.
    let (s, _) = f.api(
        "POST",
        "/api/jobs",
        json!({"kind": "capture", "run": "demo", "start": {"model": "mosshop"}}),
    );
    assert_eq!(s, 422);
}
