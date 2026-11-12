//! Edits on Moss Hop's model tree, contract operations, and the tools that
//! need no `gp` binary. crates/gp-cli/tests/mcp.rs drives the whole server
//! over stdio.

use std::path::{Path, PathBuf};

use gp_contract::{Contract, Level};
use gp_model::Model;
use serde_json::{Value, json};

use crate::edit::{self, At, ContractOp, Edit};
use crate::{Config, tools};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn mosshop() -> Model {
    Model::parse(&std::fs::read_to_string(repo().join("templates/mosshop/model.toml")).unwrap())
        .unwrap()
}

fn set(path: &str, value: Value) -> Edit {
    Edit {
        path: path.into(),
        value: Some(value),
        remove: false,
    }
}

fn remove(path: &str) -> Edit {
    Edit {
        path: path.into(),
        value: None,
        remove: true,
    }
}

/// Applies edits and parses the result, as model_edit does.
fn edited(edits: &[Edit]) -> Result<Model, String> {
    let mut tree = mosshop().to_tree();
    for e in edits {
        edit::apply(&mut tree, e)?;
    }
    Model::parse(&toml::to_string(&tree).unwrap()).map_err(|e| e.to_string())
}

#[test]
fn an_edit_changes_one_fact_and_one_line() {
    let before = mosshop();
    let after = edited(&[set("entities.player.jump.gravity", json!("0.1875"))]).unwrap();
    let changes = gp_model::diff::diff(&before.to_tree(), &after.to_tree());
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path(), "entities.player.jump.gravity");
    let (a, b) = (before.to_toml(), after.to_toml());
    let differing = a.lines().zip(b.lines()).filter(|(x, y)| x != y).count();
    assert_eq!((differing, a.lines().count()), (1, b.lines().count()));
    assert!(b.contains("gravity = \"0.1875\""));
}

#[test]
fn edits_reach_list_items_and_can_add_and_remove() {
    let m = edited(&[set("levels.main.spawn[1].x", json!(200))]).unwrap();
    assert_eq!(
        m.to_tree()["levels"]["main"]["spawn"][1]["x"].as_integer(),
        Some(200)
    );
    let n = mosshop().levels["main"].core.spawn.len();
    let spawn = mosshop().to_tree()["levels"]["main"]["spawn"][1].clone();
    let appended = edited(&[set(
        &format!("levels.main.spawn[{n}]"),
        serde_json::to_value(&spawn).unwrap(),
    )])
    .unwrap();
    assert_eq!(appended.levels["main"].core.spawn.len(), n + 1);
    let removed = edited(&[remove("levels.main.spawn[1]")]).unwrap();
    assert_eq!(removed.levels["main"].core.spawn.len(), n - 1);
}

#[test]
fn bad_edits_are_refused_with_a_reason() {
    for (e, want) in [
        (set("entities[*].size", json!([8, 8])), "is a pattern"),
        (set("evidence.x", json!(1)), "evidence table"),
        (
            set("entities.player.jump.gravity", json!(0.3)),
            "exact decimal string",
        ),
        (
            set("entities.nobody.jump.gravity", json!("0.25")),
            "`entities.nobody` isn't in the model",
        ),
        (
            set("levels.main.spawn[99].x", json!(1)),
            "`levels.main.spawn[99]` isn't in the model",
        ),
        (remove("entities.player.nope"), "isn't in the model"),
        (
            set("entities.player.jump.gravity[", json!(1)),
            "entities.player.jump.gravity[",
        ),
    ] {
        let err = edited(&[e]).unwrap_err();
        assert!(err.contains(want), "{err}");
    }
    // A key the schema doesn't know is refused by the parser.
    let err = edited(&[set("entities.player.jump.nope", json!(1))]).unwrap_err();
    assert!(err.contains("unknown field `nope`"), "{err}");
    // So is an inexact fraction.
    let err = edited(&[set("entities.player.jump.gravity", json!("0.1"))]).unwrap_err();
    assert!(err.contains("0.1"), "{err}");
}

#[test]
fn contract_ops_find_rules_by_position_or_path() {
    let text = std::fs::read_to_string(repo().join("templates/mosshop/fidelity.toml")).unwrap();
    let mut c = Contract::parse(&text).unwrap();
    let rule = c.rules[3].clone();
    let mut free = rule.clone();
    free.level = Level::Free;
    free.tolerance.clear();
    edit::apply_contract(
        &mut c,
        &[ContractOp::Set {
            at: At::Path("entities.player.jump".into()),
            rule: free,
        }],
    )
    .unwrap();
    assert_eq!(c.rules[3].level, Level::Free);
    let n = c.rules.len();
    edit::apply_contract(&mut c, &[ContractOp::Add { rule: rule.clone() }]).unwrap();
    let err = edit::apply_contract(
        &mut c,
        &[ContractOp::Remove {
            at: At::Path("entities.player.jump".into()),
        }],
    )
    .unwrap_err();
    assert!(err.contains("name one by its position"), "{err}");
    edit::apply_contract(&mut c, &[ContractOp::Remove { at: At::Index(n) }]).unwrap();
    let err = edit::apply_contract(&mut c, &[ContractOp::Remove { at: At::Index(n) }]).unwrap_err();
    assert!(err.contains("no rule"), "{err}");
}

fn config(work: &Path) -> Config {
    Config {
        gp: PathBuf::from("/nonexistent/gp"),
        repo: repo(),
        work: work.to_owned(),
        home: work.join("home"),
        rom: None,
        assets: None,
    }
}

#[test]
fn every_tool_takes_an_object() {
    for t in tools::list() {
        assert_eq!(t.input_schema["type"], "object", "{}", t.name);
        assert!(!t.name.contains('.'), "{}", t.name);
    }
    let names: Vec<_> = crate::server::tool_list()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(
        names,
        [
            "registry_search",
            "template_fork",
            "surface_describe",
            "model_edit",
            "contract_edit",
            "assets_generate",
            "assets_place",
            "assets_edit",
            "assets_history",
            "assets_export",
            "sound_banks",
            "sound_describe",
            "sound_edit",
            "sound_preview",
            "run_preview",
            "validate",
            "model_refine",
            "model_questions",
            "show_explain",
            "build_publish",
            "capture_events",
            "requests",
            "moments",
            "refine_moment",
            "watch_start",
            "watch_events",
            "watch_watch",
            "watch_hook",
            "watch_detail",
            "watch_explore",
            "watch_point",
            "view_now",
            "capture_input"
        ]
    );
    for t in tools::list() {
        assert!(
            t.input_schema["properties"]["run"].is_object(),
            "{} takes a capture run",
            t.name
        );
    }
}

#[test]
fn a_call_that_names_a_capture_run_is_logged_in_it() {
    let dir = tempfile::tempdir().unwrap();
    let c = config(dir.path());
    let run = c.home.join("capture/demo");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(run.join("run.json"), r#"{"inputs_applied": 0}"#).unwrap();
    std::fs::write(
        run.join("events.ndjson"),
        concat!(
            r#"{"v":1,"seq":1,"at_ms":0,"round":1,"actor":"pipeline","kind":"question","stage":"questions","data":{"id":"derive:levels.main.kill_below","text":"Which?","path":"levels.main.kill_below","candidates":[]}}"#,
            "\n",
            r#"{"v":1,"seq":2,"at_ms":5,"round":1,"actor":"pipeline","kind":"stage.end","stage":"validate","data":{"status":"fail","ms":5,"summary":""}}"#,
            "\n"
        ),
    )
    .unwrap();
    // A tool's own arguments never see `run`; the call is logged either way.
    let r = tools::call(
        &c,
        "surface_describe",
        serde_json::json!({"copy": "nope", "run": "demo"}),
    )
    .unwrap();
    assert!(r.is_error);
    assert!(r.json.get("capture_log").is_none(), "{}", r.json);
    let inputs = gp_capture::read_inputs(&run).unwrap();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].actor, gp_capture::Actor::Agent);
    assert_eq!(inputs[0].tool.as_deref(), Some("surface_describe"));
    assert_eq!(inputs[0].args.as_deref(), Some(r#"{"copy":"nope"}"#));
    assert!(
        inputs[0]
            .result
            .as_deref()
            .unwrap()
            .starts_with("refused: ")
    );
    // A run that isn't there is said, not fatal.
    let r = tools::call(
        &c,
        "surface_describe",
        serde_json::json!({"copy": "nope", "run": "other"}),
    )
    .unwrap();
    assert!(r.json["capture_log"].is_string(), "{}", r.json);

    let ev = tools::call(
        &c,
        "capture_events",
        serde_json::json!({"run": "demo", "after": 1}),
    )
    .unwrap();
    assert!(!ev.is_error, "{}", ev.json);
    assert_eq!(ev.json["events"].as_array().unwrap().len(), 1);
    assert_eq!(ev.json["last_seq"], 2);
    assert_eq!(
        ev.json["open_questions"][0]["id"],
        "derive:levels.main.kill_below"
    );
    assert_eq!(ev.json["pending_inputs"], 1);
    let sent = tools::call(
        &c,
        "capture_input",
        serde_json::json!({"run": "demo", "kind": "answer", "question": "derive:levels.main.kill_below", "choice": 2}),
    )
    .unwrap();
    assert!(!sent.is_error, "{}", sent.json);
    assert_eq!(sent.json["inputs"], 2);
    let bad = tools::call(
        &c,
        "capture_input",
        serde_json::json!({"run": "demo", "kind": "answer"}),
    )
    .unwrap();
    assert!(bad.is_error);
    assert_eq!(
        crate::capture::progress_line(&c, "demo", 0).unwrap(),
        (2, "round 1: validate fail".to_owned())
    );
}

#[test]
fn the_requests_tool_lists_and_replies() {
    let dir = tempfile::tempdir().unwrap();
    let c = config(dir.path());
    let run = c.home.join("capture/demo");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(run.join("run.json"), r#"{"inputs_applied": 0}"#).unwrap();
    let mut r = gp_capture::Input::new(gp_capture::Actor::Person, "request");
    r.text = Some("it should break".into());
    r.data = Some(
        serde_json::json!({"frame": 300, "recording": "walk", "choice": "react", "label": "a brick"}),
    );
    gp_capture::requests::post(&run, r).unwrap();
    let call = |args: serde_json::Value| tools::call(&c, "requests", args).unwrap();
    let l = call(serde_json::json!({"run": "demo"}));
    assert!(!l.is_error, "{}", l.json);
    assert_eq!(l.json["requests"][0]["data"]["label"], "a brick");
    assert_eq!(l.json["requests"][0]["status"], "open");
    // A proposal names its item; a reply without one is refused.
    let bad = call(
        serde_json::json!({"run": "demo", "reply": {"id": 1, "text": "x", "status": "proposed"}}),
    );
    assert!(bad.is_error);
    let ok = call(
        serde_json::json!({"run": "demo", "reply": {"id": 1, "text": "Here's a new look", "status": "proposed", "proposal": "hop:2"}}),
    );
    assert!(!ok.is_error, "{}", ok.json);
    assert_eq!(ok.json["requests"][0]["status"], "proposed");
    assert_eq!(
        ok.json["requests"][0]["replies"][0]["data"]["proposal"]["kind"],
        "asset_edit"
    );
    // The run's own argument isn't logged as a tool input.
    let inputs = gp_capture::read_inputs(&run).unwrap();
    assert!(inputs.iter().all(|i| i.kind != "tool"));
    let only = call(serde_json::json!({"run": "demo", "status": "open"}));
    assert!(only.json["requests"].as_array().unwrap().is_empty());
}

#[test]
fn every_listed_tool_is_dispatched() {
    // Arguments that don't fit any schema, so no tool does anything: each
    // listed name must reach its arm in `call` and answer with an error.
    let c = config(Path::new("/nonexistent/work"));
    for t in crate::tools::list() {
        let reply = crate::tools::call(&c, t.name, serde_json::json!([]));
        assert!(
            reply.is_some(),
            "{} is listed but `call` has no arm for it",
            t.name
        );
    }
    assert!(crate::tools::call(&c, "no_such_tool", serde_json::json!({})).is_none());
}

#[test]
fn fork_and_describe_need_no_gp() {
    let work = tempfile::tempdir().unwrap();
    let c = config(work.path());
    assert!(tools::call(&c, "no_such_tool", json!({})).is_none());
    let r = tools::call(&c, "surface_describe", json!({ "copy": "hop" })).unwrap();
    assert!(r.is_error, "{}", r.json);
    let r = tools::call(
        &c,
        "template_fork",
        json!({ "template": "mosshop", "copy": "../x" }),
    )
    .unwrap();
    assert!(
        r.is_error
            && r.json["error"]
                .as_str()
                .unwrap()
                .contains("working copy name")
    );
    let r = tools::call(
        &c,
        "template_fork",
        json!({ "template": "mosshop", "copy": "hop" }),
    )
    .unwrap();
    assert!(!r.is_error, "{}", r.json);
    let r = tools::call(
        &c,
        "template_fork",
        json!({ "template": "mosshop", "copy": "hop" }),
    )
    .unwrap();
    assert!(r.is_error, "a second fork of one name is refused");
    let r = tools::call(&c, "surface_describe", json!({ "copy": "hop", "extra": 1 })).unwrap();
    assert!(r.is_error && r.json["error"].as_str().unwrap().contains("extra"));
    let d = tools::call(&c, "surface_describe", json!({ "copy": "hop" }))
        .unwrap()
        .json;
    let facts = d["facts"].as_array().unwrap();
    let gravity = facts
        .iter()
        .find(|f| f["path"] == "entities.player.jump.gravity")
        .unwrap();
    assert_eq!(gravity["value"], "0.25");
    assert_eq!(gravity["level"], "metric");
    assert_eq!(
        gravity["tolerance"],
        json!({ "airtime": 2, "height": "5%" })
    );
    assert_eq!(gravity["evidence"]["source"], "stated");
    let by_level: usize = d["contract"]["facts_by_level"]
        .as_object()
        .unwrap()
        .values()
        .map(|n| n.as_u64().unwrap() as usize)
        .sum();
    assert_eq!(by_level, facts.len());
    assert_eq!(d["slots"].as_array().unwrap().len(), mosshop().assets.len());
    // The committed template is untouched by anything above.
    assert!(!repo().join("templates/mosshop/hop").exists());
}

#[test]
fn the_header_is_not_editable() {
    let err = edited(&[set("model.source", json!("../../tmp"))]).unwrap_err();
    assert!(err.contains("header"), "{err}");
    assert!(!crate::is_source_name("../../tmp"));
    assert!(crate::is_source_name("donkey-kong-ju"));
}

#[test]
fn removing_a_part_drops_the_evidence_that_named_only_it() {
    let before = mosshop().to_tree();
    let mut tree = before.clone();
    edit::apply(&mut tree, &remove("levels.main.goal")).unwrap();
    let dropped = edit::drop_dangling_evidence(&before, &mut tree);
    assert_eq!(dropped, ["levels.main.goal"]);
    let m = Model::parse(&toml::to_string(&tree).unwrap()).unwrap();
    assert!(!m.evidence.contains_key("levels.main.goal"));
    assert!(
        gp_model::validate::validate(&m)
            .iter()
            .all(|p| !p.message.contains("names no part")),
        "no evidence entry is left naming nothing"
    );
    // An edit that removes nothing drops nothing.
    let mut same = before.clone();
    assert!(edit::drop_dangling_evidence(&before, &mut same).is_empty());
}
