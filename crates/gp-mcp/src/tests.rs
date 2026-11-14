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
        games: None,
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
            "next",
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
            "build_publish",
            "moments",
            "watch_start",
            "watch_events",
            "watch_watch",
            "watch_hook",
            "watch_detail",
            "watch_explore",
            "watch_point",
            "view_now",
            "gp_commands",
            "gp_run"
        ]
    );
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
    assert!(r.is_error && r.json["error"].as_str().unwrap().contains("copy name"));
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

/// A copy holds the whole template folder, subfolders included, and says
/// what it still needs before it runs when the fork can't make it here
/// (no `gp` in these tests).
#[test]
fn a_fork_copies_subfolders_and_says_what_it_needs() {
    let work = tempfile::tempdir().unwrap();
    let c = config(work.path());
    let r = tools::call(
        &c,
        "template_fork",
        json!({ "template": "mosshop", "copy": "hop" }),
    )
    .unwrap();
    assert!(!r.is_error, "{}", r.json);
    let copy = work.path().join("hop");
    for f in ["level-kit/kit.toml", "level-kit/level-kit.md", "AGENTS.md"] {
        assert!(copy.join(f).is_file(), "{f} wasn't copied: {}", r.json);
        assert!(
            r.json["files"].as_array().unwrap().contains(&json!(f)),
            "{f} isn't listed: {}",
            r.json
        );
    }
    assert!(copy.join("themes").is_dir());
    // Moss Hop's ROM is built from source, which the fork doesn't do.
    assert_eq!(r.json["ready"], false, "{}", r.json);
    let next = r.json["next"].as_str().unwrap();
    assert!(
        next.contains("just mosshop") && next.contains("gp assets lift"),
        "{next}"
    );

    // A game made here: its baselines come along, and the build its assets
    // need is tried (and fails here, with no gp) and named.
    let r = tools::call(
        &c,
        "template_fork",
        json!({ "template": "badge-in", "copy": "badge" }),
    )
    .unwrap();
    assert!(!r.is_error, "{}", r.json);
    assert!(work.path().join("badge/baselines/lobby.toml").is_file());
    assert_eq!(r.json["tier"], "open");
    assert_eq!(r.json["ready"], false);
    assert!(r.json["assets"]["tried"].is_string(), "{}", r.json);
    let next = r.json["next"].as_str().unwrap();
    assert!(next.contains("gp template build"), "{next}");
    assert!(r.json.get("notes").is_none(), "{}", r.json);
}

/// A made-up game folder in a games checkout: a commercial game's
/// template, with custom code, build output, git's files and a ROM image
/// left in it by hand.
fn games_checkout(root: &Path) -> PathBuf {
    let games = root.join("games");
    let g = games.join("hopper");
    for d in ["ext/src", "level-kit", "target/debug", ".git", "inputs"] {
        std::fs::create_dir_all(g.join(d)).unwrap();
    }
    std::fs::write(
        g.join("template.toml"),
        r#"[template]
format = "0.3.1"
id = "hopper-us"
title = "Hopper"
tier = "commercial"
model = "model.toml"

[source]
id = "mosshop"
platform = "nes"
how = "rom"
accepts = ["00"]

[recipe]
asset_map = "assets/hopper.toml"

[ext]
hooks = ["hop_spring"]
crate = "ext"
"#,
    )
    .unwrap();
    std::fs::copy(
        repo().join("templates/mosshop/model.toml"),
        g.join("model.toml"),
    )
    .unwrap();
    std::fs::write(g.join("ext/src/lib.rs"), "// the hooks\n").unwrap();
    std::fs::write(g.join("level-kit/kit.toml"), "# the kit\n").unwrap();
    std::fs::write(g.join("target/debug/module.wasm"), "built").unwrap();
    std::fs::write(g.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let mut rom = b"NES\x1a".to_vec();
    rom.extend([0u8; 16]);
    std::fs::write(g.join("inputs/hopper.nes"), rom).unwrap();
    games
}

#[test]
fn a_template_in_the_games_checkout_is_found_listed_and_copied() {
    let work = tempfile::tempdir().unwrap();
    let mut c = config(work.path());
    c.games = Some(games_checkout(work.path()));
    // By the folder's name and by the template's id.
    for (template, copy) in [("hopper", "mine"), ("hopper-us", "mine2")] {
        let r = tools::call(
            &c,
            "template_fork",
            json!({ "template": template, "copy": copy }),
        )
        .unwrap();
        assert!(!r.is_error, "{}", r.json);
        assert_eq!(r.json["id"], "hopper-us");
        assert!(r.json["from"].as_str().unwrap().ends_with("games/hopper"));
    }
    let r = tools::call(
        &c,
        "template_fork",
        json!({ "template": "hopper", "copy": "mine3" }),
    )
    .unwrap();
    let copy = work.path().join("mine3");
    assert!(copy.join("ext/src/lib.rs").is_file());
    assert!(copy.join("level-kit/kit.toml").is_file());
    assert!(!copy.join("target").exists() && !copy.join(".git").exists());
    assert!(!copy.join("inputs/hopper.nes").exists());
    assert_eq!(
        r.json["left_out"][0]["path"], "inputs/hopper.nes",
        "{}",
        r.json
    );
    // A commercial game's template is copied: the copy holds nothing from
    // the ROM, and it says what that means and what it needs.
    assert_eq!(r.json["tier"], "commercial");
    assert_eq!(r.json["ready"], false);
    assert!(
        r.json["next"]
            .as_str()
            .unwrap()
            .contains("own ROM of Hopper"),
        "{}",
        r.json
    );
    let notes = r.json["notes"].to_string();
    assert!(notes.contains("build_publish refuses"), "{notes}");
    assert!(notes.contains("hop_spring"), "{notes}");
    // The copy is one the other tools take.
    let d = tools::call(&c, "surface_describe", json!({ "copy": "mine3" })).unwrap();
    assert!(!d.is_error, "{}", d.json);

    // registry_search lists it beside the repo's templates.
    let s = tools::call(&c, "registry_search", json!({ "query": "hopper" })).unwrap();
    assert!(!s.is_error, "{}", s.json);
    assert_eq!(s.json["templates"][0]["template"], "hopper", "{}", s.json);
    assert!(
        s.json["templates"][0]["folder"]
            .as_str()
            .unwrap()
            .ends_with("games/hopper")
    );
    let all = tools::call(&c, "registry_search", json!({ "limit": 50 })).unwrap();
    let names: Vec<&str> = all.json["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["template"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"mosshop") && names.contains(&"hopper"),
        "{names:?}"
    );
}

#[test]
fn a_fork_refuses_what_it_cant_copy_and_writes_nothing() {
    let work = tempfile::tempdir().unwrap();
    let mut c = config(work.path());
    // No such template: the error says where the tools looked.
    let r = tools::call(
        &c,
        "template_fork",
        json!({ "template": "hopper", "copy": "mine" }),
    )
    .unwrap();
    assert!(r.is_error);
    let err = r.json["error"].as_str().unwrap();
    assert!(
        err.contains("GP_GAMES_DIR") && err.contains("registry_search"),
        "{err}"
    );
    // A template holding a symlink isn't copied, and no copy is left.
    let games = games_checkout(work.path());
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/hosts", games.join("hopper/level-kit/hosts")).unwrap();
    c.games = Some(games);
    let r = tools::call(
        &c,
        "template_fork",
        json!({ "template": "hopper", "copy": "mine" }),
    )
    .unwrap();
    #[cfg(unix)]
    {
        assert!(r.is_error, "{}", r.json);
        let err = r.json["error"].as_str().unwrap();
        assert!(err.contains("level-kit/hosts is a symlink"), "{err}");
        assert!(!work.path().join("mine").exists());
        let left: Vec<_> = std::fs::read_dir(work.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(".mine"))
            .collect();
        assert!(left.is_empty(), "a half copy was left behind");
    }
    let _ = r;
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
