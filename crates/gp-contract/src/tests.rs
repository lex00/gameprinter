//! Contract parsing and resolution on a small model made up for the tests.

use super::*;
use gp_model::path::Step;

const MODEL: &str = r#"
[model]
schema = "0.4.0"
name = "tiny"
title = "Tiny"
view = "side"
frame_rate = "60"
extensions = { platformer = "0.4.0" }
systems = ["rules", "entity_states", "counters", "spawn", "platformer.player", "platformer.enemies", "platformer.contact", "exits", "camera"]

[input]
device = "nes-pad"
actions = { left = "left", right = "right", jump = "a" }

[assets.tiles]
kind = "tiles"
role = "graphics"

[assets.blocks]
kind = "metatiles"
role = "tileset"
item_roles = ["sky", "terrain"]

[assets.map]
kind = "level-grid"
role = "layout"

[entities.hero]
role = "player"
size = [16, 16]
hitbox = { x = 2, y = 0, w = 12, h = 16 }

[entities.hero.jump]
action = "jump"
speed = -5
gravity = "0.25"
max_fall = 4

[entities.slug]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 4, w = 16, h = 12 }

[entities.slug.patrol]
step = 1
period = 2

[entities.bat]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }

[entities.bat.patrol]
step = 2
period = 1

[levels.one]
grid = "map"
tileset = "blocks"
solid_property = "solid"
tile_size = 16
size = [16, 15]

[[levels.one.spawn]]
entity = "hero"
x = 24
y = 176

[evidence.entities]
source = "inferred"
confidence = 50

[evidence.levels]
source = "inferred"
confidence = 50

[evidence.assets]
source = "inferred"
confidence = 50

[evidence.input]
source = "inferred"
confidence = 50
"#;

const CONTRACT: &str = r#"
[contract]
schema = "0.2.0"
model = "tiny"
default = "metric"

[[rule]]
path = "levels"
level = "structural"

[[rule]]
path = "entities[role=enemy]"
level = "metric"
tolerance = { speed = "3%", timing = 2 }

[[rule]]
path = "entities.slug"
level = "metric"
tolerance = { speed = "1%" }

[[rule]]
path = "entities.hero.jump"
level = "free"

[[rule]]
path = "entities.hero.jump"
aspect = "reach"
level = "structural"

[[rule]]
path = "entities.hero.jump.gravity"
level = "exact"
note = "down to one parameter"

[[rule]]
path = "levels.one"
level = "exact"
when = "level == 1"
"#;

fn model() -> Model {
    let m = Model::parse(MODEL).unwrap();
    assert_eq!(gp_model::validate::validate(&m), Vec::new());
    m
}

fn fact(text: &str) -> Vec<Step> {
    let m = model();
    let found = Path::parse(text).unwrap().resolve(&m.to_tree());
    assert_eq!(found.len(), 1, "{text}");
    found.into_iter().next().unwrap()
}

fn problems(contract: &str) -> Vec<String> {
    match Contract::parse(contract) {
        Err(e) => vec![e.to_string()],
        Ok(c) => c.check(&model()).iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn the_contract_is_valid_and_round_trips() {
    assert_eq!(problems(CONTRACT), Vec::<String>::new());
    let c = Contract::parse(CONTRACT).unwrap();
    let text = c.to_toml();
    assert_eq!(Contract::parse(&text).unwrap(), c);
    assert_eq!(Contract::parse(&text).unwrap().to_toml(), text);
    assert!(
        text.contains("speed = \"3%\"") && text.contains("timing = 2"),
        "{text}"
    );
}

#[test]
fn most_specific_rule_wins() {
    let c = Contract::parse(CONTRACT).unwrap();
    let m = model();
    let b = c.bind(&m);
    let level = |p: &str, aspect: Option<&str>| b.resolve(&fact(p), aspect).level;

    // Inherited from a whole table.
    assert_eq!(level("levels.one.tile_size", None), Level::Structural);
    // A filter over every enemy.
    assert_eq!(level("entities.bat.patrol.step", None), Level::Metric);
    // A whole behavior, and one parameter of it.
    assert_eq!(level("entities.hero.jump.speed", None), Level::Free);
    assert_eq!(level("entities.hero.jump.gravity", None), Level::Exact);
    // An aspect: Free in general, Structural in what it reaches.
    assert_eq!(
        level("entities.hero.jump.speed", Some("reach")),
        Level::Structural
    );
    assert_eq!(level("entities.hero.jump.speed", Some("arc")), Level::Free);
    // The parameter rule is nearer than the aspect rule.
    assert_eq!(
        level("entities.hero.jump.gravity", Some("reach")),
        Level::Exact
    );
    // Nothing names the hero's size: the default.
    let r = b.resolve(&fact("entities.hero.size"), None);
    assert_eq!((r.level, r.rule), (Level::Metric, None));
    assert_eq!(r.tolerance, BTreeMap::new());
}

#[test]
fn tolerances_inherit_and_override() {
    let c = Contract::parse(CONTRACT).unwrap();
    let m = model();
    let b = c.bind(&m);
    let slug = b.resolve(&fact("entities.slug.patrol.step"), None);
    assert_eq!(slug.rule, Some(2));
    assert_eq!(slug.tolerance["speed"].to_string(), "1%");
    assert_eq!(
        slug.tolerance["timing"],
        Tolerance::Absolute(Fixed::from_int(2).unwrap())
    );
    let bat = b.resolve(&fact("entities.bat.patrol.step"), None);
    assert_eq!(bat.tolerance["speed"].to_string(), "3%");
}

#[test]
fn conditional_rules_are_listed() {
    let c = Contract::parse(CONTRACT).unwrap();
    let m = model();
    let b = c.bind(&m);
    let r = b.resolve(&fact("levels.one.tile_size"), None);
    assert_eq!(r.level, Level::Structural);
    assert_eq!(r.conditional, [6]);
    assert!(
        b.resolve(&fact("entities.hero.size"), None)
            .conditional
            .is_empty()
    );
    let s = b.summary(&m);
    assert_eq!(
        s.values().sum::<usize>(),
        gp_model::facts(&m.to_tree()).len()
    );
    assert!(s[&Level::Exact] == 1 && s[&Level::Free] >= 3, "{s:?}");
}

#[test]
fn check_reports_bad_rules_and_conflicts() {
    let with = |extra: &str| format!("{CONTRACT}\n{extra}");
    let cases = [
        (
            "[[rule]]\npath = \"entities.ghost\"\nlevel = \"free\"",
            "names no part",
        ),
        (
            "[[rule]]\npath = \"entities[\"\nlevel = \"free\"",
            "rule[7]",
        ),
        (
            "[[rule]]\npath = \"model.name\"\nlevel = \"free\"",
            "no fidelity",
        ),
        (
            "[[rule]]\npath = \"levels\"\nlevel = \"free\"\ntolerance = { x = 1 }",
            "only apply to metric",
        ),
        (
            "[[rule]]\npath = \"entities.slug\"\nlevel = \"structural\"",
            "conflicts with rule[2]",
        ),
        (
            "[[rule]]\npath = \"levels\"\nlevel = \"free\"\naspect = \"two words\"",
            "aspect name",
        ),
        (
            "[[rule]]\npath = \"levels\"\nlevel = \"free\"\nwhen = \" \"",
            "`when` is empty",
        ),
        (
            "[[rule]]\npath = \"levels\"\nlevel = \"free\"\nwhen = \"world 1\"",
            "`when` isn't a condition",
        ),
        (
            "[[rule]]\npath = \"entities.hero.size[0]\"\nlevel = \"free\"",
            "names no fact",
        ),
    ];
    for (extra, want) in cases {
        let p = problems(&with(extra));
        assert!(
            p.iter().any(|x| x.contains(want)),
            "{extra}: wanted {want}, got {p:?}"
        );
    }
    // The same level twice is no conflict.
    assert_eq!(
        problems(&with(
            "[[rule]]\npath = \"entities.slug\"\nlevel = \"metric\""
        )),
        Vec::<String>::new()
    );
    // Different `when`s don't conflict with each other.
    assert_eq!(
        problems(&with(
            "[[rule]]\npath = \"levels.one\"\nlevel = \"free\"\nwhen = \"level == 2\""
        )),
        Vec::<String>::new()
    );
    let p = problems(&CONTRACT.replace("model = \"tiny\"", "model = \"huge\""));
    assert!(p.iter().any(|x| x.contains("contract.model")), "{p:?}");
    // A 0.1.0 contract still reads: it's a 0.2.0 one with no `when`.
    assert_eq!(
        problems(&CONTRACT.replace("schema = \"0.2.0\"", "schema = \"0.1.0\"")),
        Vec::<String>::new()
    );
    let p = problems(&CONTRACT.replace("schema = \"0.2.0\"", "schema = \"1.0.0\""));
    assert!(p.iter().any(|x| x.contains("contract.schema")), "{p:?}");
    for bad in [
        "level = \"loose\"",
        "tolerance = { s = \"-1%\" }",
        "tolerance = { s = 0.5 }",
        "tolerance = { s = \"0.1\" }",
    ] {
        let text = with(&format!(
            "[[rule]]\npath = \"levels\"\n{}",
            if bad.starts_with("level") {
                bad.to_string()
            } else {
                format!("level = \"metric\"\n{bad}")
            }
        ));
        assert!(Contract::parse(&text).is_err(), "{bad}");
    }
}

#[test]
fn schema_is_json_with_its_id() {
    let v: serde_json::Value = serde_json::from_str(&schema()).unwrap();
    assert_eq!(v["$id"], "game-printer:contract@0.2.0");
    assert!(v["$defs"]["Level"].is_object());
}

#[test]
fn tied_rules_take_tolerances_from_the_first_like_the_level() {
    let text = format!(
        "{CONTRACT}\n[[rule]]\npath = \"entities.slug\"\nlevel = \"metric\"\ntolerance = {{ speed = \"50%\" }}\n"
    );
    let c = Contract::parse(&text).unwrap();
    assert_eq!(c.check(&model()), Vec::new());
    let m = model();
    let r = c.bind(&m).resolve(&fact("entities.slug.patrol.step"), None);
    assert_eq!(r.rule, Some(2));
    assert_eq!(r.tolerance["speed"].to_string(), "1%");
}

#[test]
fn the_longest_stretch_bar_is_that_many_seconds_of_the_games_own_frames() {
    let bars = |t: &str| CorpusBars {
        together: None,
        longest_stretch: Some(t.into()),
        note: None,
    };
    let rate = |t: &str| gp_model::model::Ratio::parse(t).unwrap().milli();
    let (nes, sixty, thirty) = (rate("39375000/655171"), rate("60"), rate("30"));
    for n in [1u64, 5, 10, 30] {
        let b = bars(&format!("{n}s"));
        assert_eq!(b.longest_frames(sixty), Ok(Some(n * 60)));
        assert_eq!(b.longest_frames(thirty), Ok(Some(n * 30)));
        // The NES's as the old constant, 60099, gave it: rounded up.
        assert_eq!(b.longest_frames(nes), Ok(Some((n * 60_099).div_ceil(1000))));
    }
    assert_eq!(bars("10s").longest_frames(nes), Ok(Some(601)));
    assert_eq!(bars("2.5s").longest_frames(thirty), Ok(Some(75)));
    assert!(bars("ten").longest_frames(thirty).is_err());
}
