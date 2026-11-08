//! Model parsing, canonical text, validation and diffs, on a small model
//! made up for the tests. templates/mosshop is checked by gp-cli's tests
//! and by `just check`.

use crate::diff::{Change, diff};
use crate::model::{ENTITY_CORE_FIELDS, EntityCore, LEVEL_CORE_FIELDS, LevelCore};
use crate::validate::{check_refs, evidence_for, validate};
use crate::{Fixed, Model, facts, path};

pub(crate) const SMALL: &str = r#"
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
tile_collision = true
sprite = { slot = "tiles", tiles = [0x10, 0x11, 0x12, 0x13], width = 2, palette = 0 }

[entities.hero.run]
walk_speed = 1
air_control = true

[entities.hero.jump]
action = "jump"
speed = -5
gravity = "0.25"
max_fall = 4

[entities.blob]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }

[entities.blob.patrol]
step = 1
period = 2

[levels.one]
grid = "map"
tileset = "blocks"
solid_property = "solid"
tile_size = 16
size = [16, 15]
kill_below = 208

[[levels.one.spawn]]
entity = "hero"
x = 24
y = 176

[[levels.one.spawn]]
entity = "blob"
x = 100
y = 176
params = { patrol_end = 140 }

[rules.game]
initial = "playing"

[[rules.game.states.playing.transition]]
on = "player.died"
to = "dying"

[[rules.game.states.dying.transition]]
after = 60
to = "playing"
do = ["restart-level"]

[[counters.deaths.change]]
on = "player.died"

[evidence.assets]
source = "stated"
confidence = 100
refs = [{ at = "notes.txt:1" }]

[evidence.entities]
source = "inferred"
confidence = 60

[evidence."entities.hero.jump.gravity"]
source = "stated"
confidence = 100
refs = [{ at = "notes.txt:2", quote = "gravity" }]

[evidence."entities[role=enemy]"]
source = "observed"
confidence = 90
refs = [{ at = "recording:68fdf735" }]

[evidence.levels]
source = "stated"
confidence = 100
refs = [{ at = "notes.txt:1" }]

[evidence.input]
source = "stated"
confidence = 100
refs = [{ at = "notes.txt:1" }]

[evidence.rules]
source = "stated"
confidence = 100
refs = [{ at = "notes.txt:1" }]

[evidence.counters]
source = "stated"
confidence = 100
refs = [{ at = "notes.txt:1" }]
"#;

fn small() -> Model {
    Model::parse(SMALL).unwrap()
}

fn problems(text: &str) -> Vec<String> {
    match Model::parse(text) {
        Err(e) => vec![e.to_string()],
        Ok(m) => validate(&m).iter().map(ToString::to_string).collect(),
    }
}

/// The small model with `from` replaced by `to`, which must occur once.
fn edit(from: &str, to: &str) -> String {
    assert_eq!(SMALL.matches(from).count(), 1, "{from}");
    SMALL.replace(from, to)
}

fn has(problems: &[String], text: &str) -> bool {
    problems.iter().any(|p| p.contains(text))
}

#[test]
fn the_small_model_is_valid() {
    assert_eq!(problems(SMALL), Vec::<String>::new());
    let m = small();
    let hero = &m.entities["hero"];
    assert_eq!(
        hero.platformer.jump.as_ref().unwrap().gravity,
        Fixed::parse("0.25").unwrap()
    );
    assert_eq!(hero.core.sprite.as_ref().unwrap().tiles, [16, 17, 18, 19]);
}

#[test]
fn canonical_text_is_stable_and_round_trips() {
    let m = small();
    let text = m.to_toml();
    let again = Model::parse(&text).unwrap();
    assert_eq!(again, m);
    assert_eq!(again.to_toml(), text);
    // Fractions are exact decimal strings, whole numbers integers, and
    // defaults are left out.
    assert!(text.contains("gravity = \"0.25\""), "{text}");
    assert!(text.contains("speed = -5"), "{text}");
    assert!(!text.contains("scroll_back"), "{text}");
    assert!(!text.contains("coyote_frames"), "{text}");
    assert!(text.is_ascii());
}

#[test]
fn core_field_lists_match_the_types() {
    let props = |schema: schemars::Schema| -> Vec<String> {
        let mut v: Vec<String> = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .unwrap()
            .keys()
            .cloned()
            .collect();
        v.sort();
        v
    };
    let sorted = |l: &[&str]| {
        let mut v: Vec<String> = l.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    };
    assert_eq!(
        props(schemars::schema_for!(EntityCore)),
        sorted(ENTITY_CORE_FIELDS)
    );
    assert_eq!(
        props(schemars::schema_for!(LevelCore)),
        sorted(LEVEL_CORE_FIELDS)
    );
    // And no extension reuses a core name.
    for e in crate::validate::EXTENSIONS {
        for c in e.entity_components {
            assert!(!ENTITY_CORE_FIELDS.contains(c), "{c}");
        }
        for f in e.level_fields {
            assert!(!LEVEL_CORE_FIELDS.contains(f), "{f}");
        }
    }
}

#[test]
fn type_errors_are_refused_at_parse() {
    // An unknown key on an entity names the core and extension rule.
    let p = problems(&edit(
        "tile_collision = true",
        "tile_collision = true\nwings = 2",
    ));
    assert!(has(&p, "wings") && has(&p, "declared extension"), "{p:?}");
    let p = problems(&edit("gravity = \"0.25\"", "gravity = 0.25"));
    assert!(has(&p, "fixed-point"), "{p:?}");
    let p = problems(&edit("gravity = \"0.25\"", "gravity = \"0.3\""));
    assert!(has(&p, "1/65536"), "{p:?}");
    let p = problems(&edit("view = \"side\"", "view = \"isometric\""));
    assert!(has(&p, "isometric"), "{p:?}");
    let p = problems(&edit("[rules.game]", "[rules.game]\nmood = 1"));
    assert!(has(&p, "mood"), "{p:?}");
    let m = Model::parse(&edit("frame_rate = \"60\"", "frame_rate = \"120/2\"")).unwrap();
    assert_eq!(m.model.frame_rate.to_string(), "60");
    let p = problems(&edit("frame_rate = \"60\"", "frame_rate = \"60/0\""));
    assert!(has(&p, "above zero"), "{p:?}");
}

#[test]
fn validation_finds_broken_references() {
    let cases = [
        ("schema = \"0.4.0\"", "schema = \"0.2.0\"", "model.schema"),
        (
            "{ platformer = \"0.4.0\" }",
            "{ platformer = \"9.0.0\" }",
            "model.extensions.platformer",
        ),
        (
            "{ platformer = \"0.4.0\" }",
            "{ platformer = \"0.4.0\", shmup = \"0.1.0\" }",
            "shmup",
        ),
        ("role = \"player\"", "role = \"hero\"", "entities.hero.role"),
        (
            "role = \"graphics\"",
            "role = \"sprites\"",
            "assets.tiles.role",
        ),
        (
            "item_roles = [\"sky\", \"terrain\"]",
            "item_roles = [\"sky\", \"lava\"]",
            "lava",
        ),
        (
            "slot = \"tiles\"",
            "slot = \"blocks\"",
            "entities.hero.sprite.slot",
        ),
        ("width = 2", "width = 3", "entities.hero.sprite.tiles"),
        (
            "action = \"jump\"",
            "action = \"fly\"",
            "entities.hero.jump.action",
        ),
        ("grid = \"map\"", "grid = \"tiles\"", "levels.one.grid"),
        (
            "entity = \"blob\"",
            "entity = \"ghost\"",
            "levels.one.spawn[1].entity",
        ),
        ("params = { patrol_end = 140 }", "", "needs a patrol_end"),
        (
            "params = { patrol_end = 140 }",
            "params = { patrol_end = 140, speed = 2 }",
            "speed",
        ),
        ("x = 24\ny = 176", "y = 176", "give `x`"),
        (
            "initial = \"playing\"",
            "initial = \"paused\"",
            "rules.game.initial",
        ),
        (
            "on = \"player.died\"\nto = \"dying\"",
            "on = \"player.died\"\nto = \"dead\"",
            "transition[0].to",
        ),
        (
            "after = 60",
            "after = 60\non = \"player.died\"",
            "exactly one",
        ),
        ("do = [\"restart-level\"]", "do = [\"explode\"]", "explode"),
        (
            "[[counters.deaths.change]]\non = \"player.died\"",
            "[[counters.deaths.change]]\non = \"player.sneezed\"",
            "counters.deaths.change[0].on",
        ),
        (
            "{ x = 2, y = 0, w = 12, h = 16 }",
            "{ x = 2, y = 0, w = 0, h = 16 }",
            "entities.hero.hitbox",
        ),
        ("max_fall = 4", "max_fall = -4", "max_fall"),
        ("period = 2", "period = 0", "period"),
        ("x = 24\ny = 176", "x = -1\ny = 176", "spawn[0].x"),
        ("x = 24\ny = 176", "x = 24\ny = 240", "spawn[0].y"),
        (
            "params = { patrol_end = 140 }",
            "params = { patrol_end = 90 }",
            "patrol_end",
        ),
    ];
    for (from, to, want) in cases {
        let p = problems(&edit(from, to));
        assert!(has(&p, want), "{from} -> {to}: wanted {want}, got {p:?}");
    }
    // A platformer component without the extension declared.
    let p = problems(&edit("extensions = { platformer = \"0.4.0\" }\n", ""));
    assert!(
        has(&p, "without declaring the platformer extension"),
        "{p:?}"
    );
    // Two players.
    let p = problems(&edit("entity = \"blob\"", "entity = \"hero\""));
    assert!(has(&p, "one player spawn, has 2"), "{p:?}");
}

#[test]
fn every_fact_needs_evidence() {
    let m = small();
    let by = evidence_for(&m);
    let find = |p: &str| {
        by.iter()
            .find(|(f, _)| path::display(f) == p)
            .unwrap()
            .1
            .clone()
    };
    // The most specific entry wins: a single parameter over its entity
    // type, a filter over the whole table.
    assert_eq!(
        find("entities.hero.jump.gravity").as_deref(),
        Some("entities.hero.jump.gravity")
    );
    assert_eq!(
        find("entities.hero.jump.speed").as_deref(),
        Some("entities")
    );
    assert_eq!(
        find("entities.blob.patrol.step").as_deref(),
        Some("entities[role=enemy]")
    );
    assert_eq!(
        find("levels.one.spawn[1].params.patrol_end").as_deref(),
        Some("levels")
    );

    let p = problems(&edit("[evidence.counters]", "[evidence.unused]"));
    assert!(
        has(&p, "counters.deaths.change[0].on: no evidence"),
        "{p:?}"
    );
    assert!(has(&p, "names no part of the model"), "{p:?}");
    let p = problems(&edit(
        "[evidence.levels]\nsource = \"stated\"\nconfidence = 100\nrefs = [{ at = \"notes.txt:1\" }]",
        "[evidence.levels]\nsource = \"stated\"\nconfidence = 101",
    ));
    assert!(
        has(&p, "percentage") && has(&p, "needs at least one ref"),
        "{p:?}"
    );
    let p = problems(&edit("[evidence.counters]", "[evidence.\"counters[\"]"));
    assert!(has(&p, "expected"), "{p:?}");
    let p = problems(&format!(
        "{SMALL}\n[evidence.\"entities.hero.size[0]\"]\nsource = \"inferred\"\nconfidence = 1\n"
    ));
    assert!(has(&p, "names no fact"), "{p:?}");
    let p = problems(&format!(
        "{SMALL}\n[evidence.\"entities.blob.description\"]\nsource = \"inferred\"\nconfidence = 1\n"
    ));
    assert!(has(&p, "names no part"), "{p:?}");
    let p = problems(&edit("[evidence.counters]", "[evidence.\"model.name\"]"));
    assert!(has(&p, "aren't facts"), "{p:?}");
    // Descriptions are notes, not facts.
    let with_note = edit(
        "[entities.blob]",
        "[entities.blob]\ndescription = \"a blob\"",
    );
    assert_eq!(problems(&with_note), Vec::<String>::new());
    assert!(
        !facts(&Model::parse(&with_note).unwrap().to_tree())
            .iter()
            .any(|(f, _)| path::display(f).ends_with("description"))
    );
}

#[test]
fn refs_are_checked_against_files() {
    let dir = std::env::temp_dir().join(format!("gp-model-refs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "line one\nthe gravity is 1/4\n").unwrap();
    assert_eq!(check_refs(&small(), &dir), Vec::new());

    let bad = Model::parse(&edit("quote = \"gravity\"", "quote = \"friction\"")).unwrap();
    let p: Vec<String> = check_refs(&bad, &dir)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(has(&p, "doesn't contain `friction`"), "{p:?}");
    let bad = Model::parse(&edit("at = \"notes.txt:2\"", "at = \"notes.txt:9\"")).unwrap();
    assert!(has(
        &check_refs(&bad, &dir)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        "has 2 lines"
    ));
    let bad = Model::parse(&edit("at = \"notes.txt:2\"", "at = \"../notes.txt:2\"")).unwrap();
    assert!(has(
        &check_refs(&bad, &dir)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        "inside it"
    ));
    // Something with a `/` is a file ref and needs its line.
    let noline = Model::parse(&edit("at = \"notes.txt:2\"", "at = \"dir/notes.txt\"")).unwrap();
    assert!(has(
        &check_refs(&noline, &dir)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        "path:line"
    ));
    // A recording ID that happens to be all digits isn't a file.
    let rec = Model::parse(&edit(
        "at = \"recording:68fdf735\"",
        "at = \"recording:12345678\"",
    ))
    .unwrap();
    assert_eq!(check_refs(&rec, &dir), Vec::new());
    let bad = Model::parse(&edit("at = \"notes.txt:2\"", "at = \"gone.txt:2\"")).unwrap();
    assert!(has(
        &check_refs(&bad, &dir)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        "can't read"
    ));
}

#[test]
fn diffs_name_each_change_by_path() {
    let a = small();
    assert_eq!(diff(&a.to_tree(), &a.to_tree()), Vec::new());
    // Writing the same value another way is no change.
    let same = Model::parse(&edit("speed = -5", "speed = \"-5.0\"")).unwrap();
    assert_eq!(diff(&a.to_tree(), &same.to_tree()), Vec::new());

    let b = Model::parse(
        &edit("gravity = \"0.25\"", "gravity = \"0.3125\"")
            .replace(
                "params = { patrol_end = 140 }",
                "params = { patrol_end = 150 }",
            )
            .replace("[[counters.deaths.change]]\non = \"player.died\"\n", ""),
    )
    .unwrap();
    let changes = diff(&a.to_tree(), &b.to_tree());
    let text: Vec<String> = changes.iter().map(ToString::to_string).collect();
    assert_eq!(
        text,
        [
            "- counters = {\"deaths\":{\"change\":[{\"on\":\"player.died\"}]}}",
            "~ entities.hero.jump.gravity: \"0.25\" -> \"0.3125\"",
            "~ levels.one.spawn[1].params.patrol_end: 140 -> 150",
        ]
    );
    // An evidence change is labeled by its key.
    let e = Model::parse(&edit("confidence = 60", "confidence = 70")).unwrap();
    let text: Vec<String> = diff(&a.to_tree(), &e.to_tree())
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(text, ["~ evidence.\"entities\".confidence: 60 -> 70"]);
    // Every other changed path is a model path that names the part.
    for c in &changes {
        let p = crate::Path::parse(c.path()).unwrap();
        assert!(!p.resolve(&a.to_tree()).is_empty(), "{}", c.path());
    }
    // A spawn added at the end.
    let c = Model::parse(&format!(
        "{SMALL}\n[[levels.one.spawn]]\nentity = \"blob\"\nx = 1\ny = 2\nparams = {{ patrol_end = 3 }}\n"
    ));
    // TOML refuses an array of tables appended after other tables for the
    // same key only when it's been closed; this one is still open.
    if let Ok(c) = c {
        let changes = diff(&a.to_tree(), &c.to_tree());
        assert!(
            matches!(&changes[..], [Change::Added { path, .. }] if path == "levels.one.spawn[2]"),
            "{changes:?}"
        );
    }
}

#[test]
fn generated_schemas_are_json_with_their_ids() {
    let core: serde_json::Value = serde_json::from_str(&crate::schema::core()).unwrap();
    assert_eq!(core["$id"], "game-printer:model-core@0.4");
    assert_eq!(core["additionalProperties"], false);
    assert!(core["$defs"]["Entity"]["additionalProperties"] == true);
    let pl: serde_json::Value = serde_json::from_str(&crate::schema::platformer()).unwrap();
    assert_eq!(pl["$id"], "game-printer:platformer@0.4");
    assert!(pl["$defs"]["Jump"]["properties"]["gravity"].is_object());
    let td: serde_json::Value = serde_json::from_str(&crate::schema::topdown()).unwrap();
    assert_eq!(td["$id"], "game-printer:topdown@0.1");
    assert!(td["$defs"]["Move8"]["properties"]["speed"].is_object());
}

// Core 0.2.0 and platformer 0.2.0: what Super Mario Bros. needs.

/// A model shaped like Super Mario Bros.: levels built from object lists,
/// an entity state machine, guards, counters that tick and wrap, jump
/// bands and walking enemies.
const SHAPED: &str = r#"
[model]
schema = "0.4.0"
name = "shaped"
title = "Shaped"
view = "side"
frame_rate = "60"
systems = ["rules", "entity_states", "counters", "spawn", "platformer.player", "platformer.enemies", "platformer.contact", "exits", "camera"]
start_level = "one"
extensions = { platformer = "0.4.0" }

[input]
device = "nes-pad"
actions = { left = "left", right = "right", jump = "a", run = "b", down = "down" }

[assets.areas]
kind = "level-objects"
role = "layout"

[assets.enemies]
kind = "spawn-objects"
role = "spawns"
entities = { walker = [6], shell = [0, 1] }

[entities.hero]
role = "player"
size = [16, 16]
hitbox = { x = 3, y = 2, w = 10, h = 14 }
tile_collision = true

[entities.hero.machine]
initial = "small"

[entities.hero.machine.states.small]
[[entities.hero.machine.states.small.transition]]
on = "item.collected"
to = "big"

[[entities.hero.machine.states.small.transition]]
on = "player.hurt"
to = "small"
do = ["kill-player"]

[entities.hero.machine.states.big]
set = { size = [16, 32], hitbox = { y = 4, h = 28 } }

[[entities.hero.machine.states.big.transition]]
on = "player.hurt"
to = "small"

[entities.hero.run]
walk_speed = "1.5"
run_speed = "2.5"
run_action = "run"
acceleration = "0.037109375"
deceleration = "0.05078125"
skid_deceleration = "0.1015625"
air_control = true

[entities.hero.jump]
action = "jump"
speed = -5
gravity = "0.5625"
gravity_held = "0.15625"
max_fall = 4
bands = [
  { below = "0.5625", speed = -4, gravity = "0.4375", gravity_held = "0.125" },
  { below = "2.3125", speed = -4, gravity = "0.375", gravity_held = "0.1171875" },
]

[entities.walker]
role = "enemy"
size = [16, 16]
hitbox = { x = 2, y = 4, w = 12, h = 12 }
tile_collision = true
walk = { speed = "0.5" }
fall = { gravity = "0.25", max_fall = 4 }
contact = { stomp_depth = 8, bounce = -4, keep_on_stomp = true, otherwise = "hurt-player" }

[entities.walker.machine]
initial = "walking"

[[entities.walker.machine.states.walking.transition]]
on = "enemy.stomped"
to = "flat"

[[entities.walker.machine.states.flat.transition]]
after = 30
to = "flat"
do = ["remove"]

[entities.walker.machine.states.flat]
set = { walk = { speed = 0 }, contact = { otherwise = "nothing" } }

[entities.shell]
role = "enemy"
size = [16, 24]
hitbox = { x = 2, y = 8, w = 12, h = 16 }
walk = { speed = "0.5", turn_at_ledges = true }
fall = { gravity = "0.25", max_fall = 4 }
contact = { otherwise = "kick" }

[levels.one]
layout = { slot = "areas", entry = "ground-6" }
spawn_objects = { slot = "enemies", entry = "ground-6" }
tile_size = 16
size = [224, 15]
spawn_ahead = 32
despawn_behind = 64
counters = { time = 400 }
next = "two"
exit = [{ to = "bonus", via = "pipe", take = { at = { x = 912, y = 112, w = 32, h = 4 }, hold = "down", standing = true }, enter = { x = 24, y = 32 } }]

[[levels.one.spawn]]
entity = "hero"
x = 40
y = 192

[levels.two]
layout = { slot = "areas", entry = "underground-1" }
tile_size = 16
size = [200, 15]
counters = { time = 300 }

[[levels.two.spawn]]
entity = "hero"
x = 40
y = 192

[levels.bonus]
layout = { slot = "areas", entry = "underground-3" }
tile_size = 16
size = [16, 15]
exit = [{ to = "one", via = "pipe" }]

[[levels.bonus.spawn]]
entity = "hero"
x = 40
y = 32

[cameras.main]
follow = "hero"
axis = "x"
size = [256, 240]
lead = [112, 0]
clamp_to_level = true
scroll_back = false

[rules.game]
initial = "playing"

[[rules.game.states.playing.transition]]
on = "player.died"
to = "dying"

[[rules.game.states.playing.transition]]
on = "goal.reached"
to = "playing"
do = ["next-level"]

[[rules.game.states.dying.transition]]
after = 180
to = "playing"
if = { counter = "lives", cmp = ">=", value = 0 }
do = ["restart-level"]

[[rules.game.states.dying.transition]]
after = 180
to = "playing"
do = ["reset-game"]

[rules.game.states.dying]
freeze = true

[counters.lives]
start = 2
change = [{ on = "player.died", by = -1 }, { on = "coins.wrapped" }]

[counters.coins]
max = 99
wrap = true
change = [{ on = "item.collected" }]

[counters.time]
min = 0
change = [{ every = 24, by = -1 }]

[counters.score]
change = [{ on = "enemy.stomped", by = 100 }]

[evidence.assets]
source = "inferred"
confidence = 50
[evidence.entities]
source = "inferred"
confidence = 50
[evidence.levels]
source = "inferred"
confidence = 50
[evidence.input]
source = "inferred"
confidence = 50
[evidence.cameras]
source = "inferred"
confidence = 50
[evidence.rules]
source = "inferred"
confidence = 50
[evidence.counters]
source = "stated"
confidence = 100
refs = [{ at = "SMBDIS.ASM:1", in = "smbdis@7e8e9777", quote = "anything" }]
"#;

fn shaped_edit(from: &str, to: &str) -> String {
    assert_eq!(SHAPED.matches(from).count(), 1, "{from}");
    SHAPED.replace(from, to)
}

#[test]
fn the_shaped_model_is_valid() {
    assert_eq!(problems(SHAPED), Vec::<String>::new());
    let m = Model::parse(SHAPED).unwrap();
    // Canonical text round-trips, `set` values included.
    let text = m.to_toml();
    assert_eq!(Model::parse(&text).unwrap(), m);
    assert_eq!(Model::parse(&text).unwrap().to_toml(), text);
    assert_eq!(m.levels["two"].core.counters["time"], 300);
}

#[test]
fn an_entity_state_merges_its_set() {
    let m = Model::parse(SHAPED).unwrap();
    let hero = &m.entities["hero"];
    let big = hero.in_state("big").unwrap();
    assert_eq!(
        big.core.size,
        [Fixed::from_int(16).unwrap(), Fixed::from_int(32).unwrap()]
    );
    // The hitbox merges key by key: x and w stay.
    assert_eq!(big.core.hitbox.x, Fixed::from_int(3).unwrap());
    assert_eq!(big.core.hitbox.h, Fixed::from_int(28).unwrap());
    assert_eq!(hero.in_state("small").unwrap(), *hero);
    assert!(hero.in_state("fire").is_err());
    let flat = m.entities["walker"].in_state("flat").unwrap();
    assert_eq!(flat.platformer.walk.as_ref().unwrap().speed, Fixed::ZERO);
    assert_eq!(
        flat.platformer.contact.as_ref().unwrap().otherwise,
        crate::platformer::Touch::Nothing
    );
    // A set value written another way is canonical after parsing.
    let other = Model::parse(&shaped_edit(
        "walk = { speed = 0 }",
        "walk = { speed = \"0.0\" }",
    ))
    .unwrap();
    assert_eq!(other.to_toml(), m.to_toml());
    // Paths into a state's set are facts with evidence.
    let tree = m.to_tree();
    let fs: Vec<String> = facts(&tree).iter().map(|(f, _)| path::display(f)).collect();
    assert!(
        fs.contains(&"entities.walker.machine.states.flat.set.walk.speed".to_owned()),
        "{fs:?}"
    );
}

#[test]
fn core_two_finds_broken_parts() {
    let cases = [
        (
            "\"spawn\", \"platformer.player\"",
            "\"platformer.player\"",
            "add `spawn`",
        ),
        (
            "\"rules\", \"entity_states\"",
            "\"rules\", \"rules\", \"entity_states\"",
            "listed twice",
        ),
        (
            "\"camera\"]",
            "\"camera\", \"physics\"]",
            "`physics` isn't a system",
        ),
        ("start_level = \"one\"\n", "", "model.start_level"),
        (
            "start_level = \"one\"",
            "start_level = \"zero\"",
            "model.start_level",
        ),
        (
            "entities = { walker = [6], shell = [0, 1] }",
            "entities = { walker = [6], shell = [6] }",
            "places more than one",
        ),
        (
            "entities = { walker = [6], shell = [0, 1] }",
            "entities = { ghost = [6] }",
            "assets.enemies.entities.ghost",
        ),
        (
            "set = { size = [16, 32], hitbox = { y = 4, h = 28 } }",
            "set = { size = [16, 32], hitbox = { y = \"x\" } }",
            "machine.states.big.set",
        ),
        (
            "set = { size = [16, 32], hitbox = { y = 4, h = 28 } }",
            "set = { role = \"enemy\" }",
            "can't set `role`",
        ),
        (
            "set = { size = [16, 32], hitbox = { y = 4, h = 28 } }",
            "set = { wings = 2 }",
            "machine.states.big.set",
        ),
        ("do = [\"remove\"]", "do = [\"explode\"]", "explode"),
        (
            "if = { counter = \"lives\"",
            "if = { counter = \"hearts\"",
            "if.counter",
        ),
        (
            "[counters.time]\nmin = 0\nchange = [{ every = 24, by = -1 }]",
            "[counters.time]\nmin = 0\nchange = [{ every = 0, by = -1 }]",
            "every",
        ),
        (
            "{ every = 24, by = -1 }",
            "{ every = 24, on = \"level.started\" }",
            "exactly one",
        ),
        (
            "{ on = \"coins.wrapped\" }",
            "{ on = \"coins.reached_min\" }",
            "coins.reached_min",
        ),
        (
            "max = 99\nwrap = true",
            "wrap = true",
            "wrapping needs a max",
        ),
        (
            "layout = { slot = \"areas\", entry = \"ground-6\" }",
            "",
            "exactly one of grid and layout",
        ),
        (
            "layout = { slot = \"areas\", entry = \"ground-6\" }",
            "layout = { slot = \"enemies\", entry = \"ground-6\" }",
            "levels.one.layout.slot",
        ),
        (
            "spawn_objects = { slot = \"enemies\"",
            "spawn_objects = { slot = \"areas\"",
            "levels.one.spawn_objects.slot",
        ),
        ("next = \"two\"", "next = \"three\"", "levels.one.next"),
        (
            "[{ to = \"one\", via = \"pipe\" }]",
            "[{ to = \"nowhere\", via = \"pipe\" }]",
            "exit[0].to",
        ),
        (
            "counters = { time = 400 }",
            "counters = { clock = 400 }",
            "levels.one.counters.clock",
        ),
        (
            "counters = { time = 400 }",
            "counters = { time = -1 }",
            "outside the counter's min",
        ),
        (
            "walk = { speed = 0 }, contact",
            "walk = { speed = -1 }, contact",
            "machine.states.flat.set",
        ),
        ("spawn_ahead = 32", "spawn_ahead = -32", "spawn_ahead"),
        (
            "{ below = \"2.3125\"",
            "{ below = \"0.5\"",
            "bands[1].below",
        ),
        (
            "contact = { otherwise = \"kick\" }",
            "contact = { keep_on_stomp = true, otherwise = \"kick\" }",
            "keep_on_stomp",
        ),
        (
            "walk = { speed = \"0.5\", turn_at_ledges = true }",
            "walk = { speed = \"-0.5\" }",
            "walk.speed",
        ),
        ("in = \"smbdis@7e8e9777\"", "in = \"smbdis\"", "refs[0].in"),
        (
            "[entities.hero.machine]",
            "sprite = { slot = \"areas\", frame = { slot = \"areas\", row = 0 }, width = 2, palette = 0 }\n\n[entities.hero.machine]",
            "entities.hero.sprite.frame.slot",
        ),
        (
            "counters = { time = 400 }",
            "counters = { time = 400 }\npresent = { palettes = \"areas\" }",
            "levels.one.present.palettes",
        ),
        (
            "do = [\"remove\"]",
            "do = [\"change:hearts:1\"]",
            "no counter named `hearts`",
        ),
        (
            "do = [\"remove\"]",
            "do = [\"change:lives\"]",
            "isn't `change:",
        ),
        ("hold = \"down\"", "hold = \"dive\"", "take.hold"),
        (
            "do = [\"remove\"]",
            "do = [\"become:hero\"]",
            "can't become the player",
        ),
        (
            "enter = { x = 24, y = 32 } }]",
            "enter = { x = 24, y = 32 }, start = true }]",
            "starts its level",
        ),
        (
            "enter = { x = 24, y = 32 }",
            "enter = { x = 9999, y = 32 }",
            "exit[0].enter",
        ),
        (
            "do = [\"remove\"]",
            "do = [\"become:ghost\"]",
            "no entity named `ghost`",
        ),
        (
            "walk = { speed = \"0.5\", turn_at_ledges = true }",
            "path = { axis = \"y\", distance = 0, speed = 1 }",
            "path.distance",
        ),
    ];
    for (from, to, want) in cases {
        let p = problems(&shaped_edit(from, to));
        assert!(has(&p, want), "{from} -> {to}: wanted {want}, got {p:?}");
    }
    // A fall and a jump on one entity.
    let p = problems(&shaped_edit(
        "tile_collision = true\n\n[entities.hero.machine]",
        "tile_collision = true\nfall = { gravity = 1, max_fall = 4 }\n\n[entities.hero.machine]",
    ));
    assert!(has(&p, "give one of jump and fall"), "{p:?}");
    // `set` only in an entity's states.
    let p = problems(&shaped_edit(
        "[rules.game.states.dying]\nfreeze = true",
        "[rules.game.states.dying]\nfreeze = true\nset = { size = 1 }",
    ));
    assert!(has(&p, "only an entity's state sets fields"), "{p:?}");
    // `remove` only in an entity's machine.
    let p = problems(&shaped_edit("do = [\"reset-game\"]", "do = [\"remove\"]"));
    assert!(has(&p, "`remove` isn't an action"), "{p:?}");
}

#[test]
fn guards_compare() {
    use crate::model::Cmp;
    assert!(Cmp::Ge.holds(0, 0) && !Cmp::Gt.holds(0, 0) && Cmp::Ne.holds(1, 0));
    assert!(Cmp::Lt.holds(-1, 0) && Cmp::Le.holds(0, 0) && Cmp::Eq.holds(3, 3));
}

#[test]
fn refs_in_a_pinned_download_are_left_to_it() {
    let m = Model::parse(SHAPED).unwrap();
    assert_eq!(
        check_refs(&m, std::path::Path::new("/nonexistent")),
        Vec::new()
    );
}

#[test]
fn refs_in_a_pinned_download_are_read_from_its_root() {
    use crate::validate::{RefRoot, check_refs_in};
    let m = Model::parse(SHAPED).unwrap();
    let dir = std::env::temp_dir().join(format!("gp-model-pinned-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SMBDIS.ASM"), "nothing here\n").unwrap();
    let (p, skipped) = check_refs_in(&m, |within, _| match within {
        Some("smbdis@7e8e9777") => RefRoot::Dir(dir.clone()),
        _ => RefRoot::Bad("unexpected".into()),
    });
    assert_eq!(skipped, 0);
    assert!(
        p.iter()
            .any(|p| p.message.contains("doesn't contain `anything`")),
        "{p:?}"
    );
    std::fs::write(dir.join("SMBDIS.ASM"), "anything at all\n").unwrap();
    let (p, _) = check_refs_in(&m, |_, _| RefRoot::Dir(dir.clone()));
    assert_eq!(p, Vec::new());
    let (p, _) = check_refs_in(&m, |_, _| RefRoot::Bad("no such download".into()));
    assert!(
        p.iter().any(|p| p.message.contains("no such download")),
        "{p:?}"
    );
    let (p, skipped) = check_refs_in(&m, |_, _| RefRoot::Skip);
    assert_eq!((p.len(), skipped), (0, 1));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// An entity's `ext_params` are facts of the entity: they need its `ext`,
/// evidence covers them as any other part, and a state's `set` changes them.
#[test]
fn ext_params_belong_to_the_hook() {
    assert!(small().entities["blob"].core.ext_params.is_empty());
    let entity = "[entities.blob]";
    let without_ext = edit(
        entity,
        &format!("{entity}\next_params = {{ speed = \"0.5\" }}"),
    );
    assert!(
        has(
            &problems(&without_ext),
            "only an entity with an `ext` hook has ext_params"
        ),
        "{:?}",
        problems(&without_ext)
    );
    let with_ext = edit(
        entity,
        &format!("{entity}\next = \"hook\"\next_params = {{ speed = \"0.5\" }}"),
    );
    let m = Model::parse(&with_ext).unwrap();
    let e = m.entities.values().find(|e| e.core.ext.is_some()).unwrap();
    assert_eq!(e.core.ext_params["speed"], Fixed::parse("0.5").unwrap());
    let tree = m.to_tree();
    assert!(
        facts(&tree)
            .iter()
            .any(|(p, _)| path::display(p).ends_with(".ext_params.speed")),
        "ext_params are facts"
    );
    // The canonical text keeps them, and reads back the same.
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
}

/// Core 0.4.0 and platformer 0.4.0: the parts Mega Man 2 and Super Mario
/// Bros. 2 needed, on the shaped model.
fn core_four() -> String {
    let s = shaped_edit(
        "start_level = \"one\"\n",
        "start_level = \"one\"\nevents = [\"hero.waved\"]\n",
    );
    let s = s.replace(
        "actions = { left = \"left\", right = \"right\", jump = \"a\", run = \"b\", down = \"down\" }",
        "actions = { left = \"left\", right = \"right\", jump = \"a\", run = \"b\", down = \"down\", up = \"up\" }",
    );
    let s = s.replace(
        "[entities.hero.run]\n",
        "[entities.hero.climb]\nspeed = \"0.75\"\nup = \"up\"\ndown = \"down\"\nrelease = \"jump\"\nstand_on_top = true\n\n\
         [entities.hero.health]\ncounter = \"lives\"\ninvulnerable = 111\n\n\
         [entities.hero.knockback]\nspeed = \"0.5625\"\nrise = \"-1.25\"\nframes = 27\n\n\
         [entities.hero.shoot]\nentity = \"shell\"\naction = \"run\"\nahead = [16, 0]\nmax = 3\n\n\
         [entities.hero.carry]\naction = \"run\"\nlift_frames = 12\noffset = [0, -16]\nthrow = [2, -1]\n\n\
         [entities.hero.run]\nstart = { speed = \"0.125\", frames = 7 }\nair_speed = \"1.3125\"\n",
    );
    let s = s.replace(
        "contact = { stomp_depth = 8, bounce = -4, keep_on_stomp = true, otherwise = \"hurt-player\" }",
        "contact = { stomp_depth = 8, stand = true, otherwise = \"hurt-player\", damage = 4 }\ncarryable = {}\nhealth = { points = 20, damage_from = { shell = 10 } }",
    );
    let s = s.replace(
        "contact = { otherwise = \"kick\" }",
        "contact = { otherwise = \"kick\" }\nhits = { damage = 2 }",
    );
    let s = s.replace(
        "despawn_behind = 64\n",
        "despawn_behind = 64\ndespawn_ahead = 8\nrespawn = true\norigin = [0, 0]\ncamera = { axis = \"y\", step = 4, hold_player = true }\nhazard = { touch = \"kill-player\" }\n",
    );
    s.replace(
        "exit = [{ to = \"bonus\", via = \"pipe\", take = { at = { x = 912, y = 112, w = 32, h = 4 }, hold = \"down\", standing = true }, enter = { x = 24, y = 32 } }]",
        "exit = [{ to = \"bonus\", via = \"pipe\", take = { at = { x = 912, y = 112, w = 32, h = 4 }, hold = \"down\", standing = true, if = { counter = \"coins\", cmp = \">\", value = 0 } }, enter = { x = 24, y = 32 }, keep = \"y\", scroll = 126 }]",
    )
}

#[test]
fn core_four_parts_parse_validate_and_round_trip() {
    let text = core_four();
    assert_eq!(problems(&text), Vec::<String>::new());
    let m = Model::parse(&text).unwrap();
    let again = Model::parse(&m.to_toml()).unwrap();
    assert_eq!(m, again);
    assert_eq!(m.model.events, ["hero.waved"]);
    let hero = &m.entities["hero"].platformer;
    assert!(hero.climb.as_ref().unwrap().stand_on_top);
    assert_eq!(hero.shoot.as_ref().unwrap().max, 3);
    let walker = &m.entities["walker"].platformer;
    assert!(walker.contact.as_ref().unwrap().stand);
    assert_eq!(
        m.entities["shell"].platformer.hits.as_ref().unwrap().roles,
        ["enemy", "boss"]
    );
}

#[test]
fn core_four_finds_broken_parts() {
    let base = core_four();
    let cases = [
        (
            "events = [\"hero.waved\"]",
            "events = [\"level.started\"]",
            "already an event",
        ),
        (
            "events = [\"hero.waved\"]",
            "events = [\"waved\"]",
            "dotted event name",
        ),
        (
            "health = { points = 20,",
            "health = { counter = \"lives\",",
            "keeps its own hit points",
        ),
        (
            "entity = \"shell\"\naction",
            "entity = \"nobody\"\naction",
            "no entity named `nobody`",
        ),
        ("step = 4, hold_player", "hold_player", "give step"),
        (
            "cmp = \">\", value = 0 }",
            "cmp = \">\", value = 0, counter = \"x\" }",
            "duplicate",
        ),
        (
            "hits = { damage = 2 }",
            "hits = { roles = [\"wall\"] }",
            "isn't an entity role",
        ),
        (
            "hazard = { touch = \"kill-player\" }",
            "hazard = { touch = \"kick\" }",
            "kills or hurts",
        ),
    ];
    for (from, to, want) in cases {
        assert_eq!(base.matches(from).count(), 1, "{from}");
        let text = base.replace(from, to);
        let found = match Model::parse(&text) {
            Err(e) => vec![e.to_string()],
            Ok(_) => problems(&text),
        };
        assert!(has(&found, want), "{to}: {found:?}");
    }
}

#[test]
fn item_roles_are_required_from_core_0_3_1() {
    // Core 0.4.0 keeps 0.3.1's rule: a tileset without item roles is
    // refused at the slot.
    let without = SMALL.replace("item_roles = [\"sky\", \"terrain\"]\n", "");
    assert_ne!(without, SMALL);
    let m = Model::parse(&without).unwrap();
    let problems = validate(&m);
    assert!(
        problems
            .iter()
            .any(|p| p.path == "assets.blocks.item_roles" && p.message.contains("0.3.1")),
        "{problems:?}"
    );
    // With them, it's valid.
    let m = Model::parse(SMALL).unwrap();
    assert!(validate(&m).is_empty(), "{:?}", validate(&m));
    assert!(crate::validate::version_at_least(
        crate::CORE_VERSION,
        crate::validate::ITEM_ROLES_FROM
    ));
}

#[test]
fn roles_fall_in_groups() {
    use crate::validate::{ENTITY_ROLES, WORLD_ROLES, role_group};
    for r in ENTITY_ROLES.iter().chain(WORLD_ROLES) {
        // `font` is a world role and a slot role; every other is in a group.
        assert!(role_group(r).is_some(), "{r} has no group");
    }
    assert_eq!(role_group("player"), Some("actors"));
    assert_eq!(role_group("hazard"), Some("things"));
    assert_eq!(role_group("sky"), Some("world"));
    assert_eq!(role_group("hud"), Some("interface"));
    assert_eq!(role_group("sfx.jump"), Some("sound"));
    assert_eq!(role_group("layout"), None);
}

#[test]
fn sound_slots_and_bindings_are_checked() {
    let sound = r#"[assets.songs]
kind = "songs"
role = "music.level"
capture = { recording = "68fdf735", frames = "1..2400", name = "one" }

[assets.effects]
kind = "sound-effects"
role = "sfx.game"

[present.sound]
music = [{ slot = "songs", song = "theme", level = "one" }, { slot = "songs", song = "death", state = "game.dying" }]
sfx = [{ event = "player.died", slot = "effects", effect = "ouch" }]

[evidence.present]
source = "observed"
confidence = 90
refs = [{ at = "recording:68fdf735" }]

[entities.hero]"#;
    let good = edit("[entities.hero]", sound);
    assert_eq!(problems(&good), Vec::<String>::new());
    let bad = good
        .replace("level = \"one\" }", "level = \"two\" }")
        .replace("state = \"game.dying\"", "state = \"game.sleeping\"")
        .replace("event = \"player.died\"", "event = \"player.sneezed\"")
        .replace("slot = \"effects\"", "slot = \"songs\"")
        .replace("role = \"sfx.game\"", "role = \"graphics\"");
    let p = problems(&bad);
    for want in [
        "no level named `two`",
        "no rule state `game.sleeping`",
        "`player.sneezed` isn't an event",
        "is a songs slot, not sound-effects",
        "a sound slot's role is music.<mood> or sfx.<event>",
    ] {
        assert!(has(&p, want), "{want}: {p:?}");
    }
}

/// Platformer 0.4.2's run options parse, validate and round-trip; each
/// needs the parts it works with; a model without them is as before.
#[test]
fn run_options_of_0_4_2_parse_and_validate() {
    let with = |extra: &str| {
        shaped_edit(
            "[entities.hero.run]\n",
            &format!("[entities.hero.run]\n{extra}"),
        )
        .replace("platformer = \"0.4.0\"", "platformer = \"0.4.2\"")
    };
    let all = with(
        "move_step = \"0.0625\"\ncap_whole = true\nskid_by = \"last_motion\"\nskid_turn_below = \"0.6875\"\n\
         decel_bands = [{ from = \"2.0625\", rate = \"0.05078125\" }]\ndecel_hold_from = \"1.75\"\n\
         run_hold_frames = 10\nrun_with_motion = true\n",
    )
    .replace("skid_deceleration = \"0.1015625\"\n", "");
    assert_eq!(problems(&all), Vec::<String>::new());
    let m = Model::parse(&all).unwrap();
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let r = m.entities["hero"].platformer.run.as_ref().unwrap();
    assert_eq!(r.skid_by, crate::platformer::SkidBy::LastMotion);
    assert_eq!(r.run_hold_frames, 10);
    assert_eq!(r.decel_bands.len(), 1);
    // Off by default: the shaped model has none of them.
    let plain = Model::parse(SHAPED).unwrap();
    let r = plain.entities["hero"].platformer.run.as_ref().unwrap();
    assert!(r.move_step.is_none() && !r.cap_whole && r.decel_bands.is_empty());
    assert_eq!(r.skid_by, crate::platformer::SkidBy::Speed);
    assert!(!m.to_toml().is_empty() && !plain.to_toml().contains("skid_by"));
    // Each needs its parts.
    let bad = |extra: &str, want: &str| {
        let p = problems(&with(extra));
        assert!(p.iter().any(|x| x.contains(want)), "{extra}: {p:?}");
    };
    bad("cap_whole = true\n", "needs move_step");
    bad("skid_by = \"last_motion\"\n", "leave skid_deceleration out");
    bad("decel_hold_from = \"1\"\n", "needs decel_bands");
    bad("move_step = \"0\"\n", "move_step: must be above zero");
    bad(
        "decel_bands = [{ from = \"2\", rate = \"0.125\" }, { from = \"1\", rate = \"0.25\" }]\n",
        "each above the last",
    );
}

#[test]
fn a_camera_by_move_of_0_4_4_parses_and_validates() {
    let with = |extra: &str, schema: &str| {
        shaped_edit(
            "scroll_back = false\n",
            &format!("scroll_back = false\n{extra}"),
        )
        .replace("schema = \"0.4.0\"", &format!("schema = \"{schema}\""))
    };
    let ok = with(
        "by_move = { from = 80, full = 112, short_by = 1 }\n",
        "0.4.4",
    );
    assert_eq!(problems(&ok), Vec::<String>::new());
    let m = Model::parse(&ok).unwrap();
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let b = m.cameras["main"].by_move.unwrap();
    assert_eq!(
        (b.from, b.full),
        (
            crate::Fixed::from_int(80).unwrap(),
            crate::Fixed::from_int(112).unwrap()
        )
    );
    // Off by default.
    assert!(
        Model::parse(SHAPED).unwrap().cameras["main"]
            .by_move
            .is_none()
    );
    let bad = |text: &str, want: &str| {
        let p = problems(text);
        assert!(p.iter().any(|x| x.contains(want)), "{want}: {p:?}");
    };
    // An older patch in `schema` doesn't refuse it: options are features.
    assert!(
        !problems(&with(
            "by_move = { from = 80, full = 112, short_by = 1 }\n",
            "0.4.3",
        ))
        .iter()
        .any(|x| x.contains("core schema"))
    );
    bad(
        &with(
            "by_move = { from = 120, full = 112, short_by = 1 }\n",
            "0.4.4",
        ),
        "must go in order",
    );
    bad(
        &with(
            "by_move = { from = 80, full = 112, short_by = -1 }\n",
            "0.4.4",
        ),
        "can't be below zero",
    );
    bad(
        &with(
            "by_move = { from = 80, full = 112, short_by = 1 }\n",
            "0.4.4",
        )
        .replace("scroll_back = false\n", ""),
        "needs scroll_back = false",
    );
}

/// Platformer 0.4.3's fall limit from one half past it and the run's air
/// options parse, validate and round-trip; each needs its parts; a model
/// without them is as before.
#[test]
fn air_options_of_0_4_3_parse_and_validate() {
    let with = |run: &str, jump: &str| {
        shaped_edit(
            "[entities.hero.run]\n",
            &format!("[entities.hero.run]\n{run}"),
        )
        .replace(
            "[entities.hero.jump]\n",
            &format!("[entities.hero.jump]\n{jump}"),
        )
        .replace("platformer = \"0.4.0\"", "platformer = \"0.4.3\"")
    };
    let all = with(
        "takeoff_in_air = true\nface_on_ground = true\nair_run_from = \"1.5625\"\n",
        "max_fall_from_half = true\n",
    );
    assert_eq!(problems(&all), Vec::<String>::new());
    let m = Model::parse(&all).unwrap();
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let hero = &m.entities["hero"].platformer;
    let r = hero.run.as_ref().unwrap();
    assert!(r.takeoff_in_air && r.face_on_ground && r.air_run_from.is_some());
    assert!(hero.jump.as_ref().unwrap().max_fall_from_half);
    // Off by default.
    let plain = Model::parse(SHAPED).unwrap();
    let hero = &plain.entities["hero"].platformer;
    let r = hero.run.as_ref().unwrap();
    assert!(!r.takeoff_in_air && !r.face_on_ground && r.air_run_from.is_none());
    assert!(!hero.jump.as_ref().unwrap().max_fall_from_half);
    assert!(!plain.to_toml().contains("max_fall_from_half"));
    let bad = |run: &str, jump: &str, want: &str| {
        let p = problems(&with(run, jump));
        assert!(p.iter().any(|x| x.contains(want)), "{run}{jump}: {p:?}");
    };
    bad(
        "air_run_from = \"0\"\n",
        "",
        "air_run_from: must be above zero",
    );
    let half = with("", "max_fall_from_half = true\n")
        .replace("max_fall = 4\nbands", "max_fall = \"4.5\"\nbands");
    assert!(
        problems(&half).iter().any(|x| x.contains("must be whole")),
        "{:?}",
        problems(&half)
    );
    let no_run = with("air_run_from = \"1\"\n", "")
        .replace("run_speed = \"2.5\"\nrun_action = \"run\"\n", "");
    assert!(
        problems(&no_run)
            .iter()
            .any(|x| x.contains("air_run_from") && x.contains("needs run_speed")),
        "{:?}",
        problems(&no_run)
    );
}

/// Platformer 0.4.5's frame on the ground without gravity and the landing
/// that keeps the height's fraction parse, validate and round-trip, and
/// are off by default.
#[test]
fn no_gravity_on_ground_of_0_4_5_parses_and_validates() {
    let on = shaped_edit(
        "[entities.hero.jump]\n",
        "[entities.hero.jump]\nno_gravity_on_ground = true\nland_keeps_fraction = true\nland_depth = 5\nfeet = [3, 12]\n",
    )
    .replace("platformer = \"0.4.0\"", "platformer = \"0.4.5\"");
    assert_eq!(problems(&on), Vec::<String>::new());
    let m = Model::parse(&on).unwrap();
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let jump = |m: &Model| m.entities["hero"].platformer.jump.clone().unwrap();
    assert!(jump(&m).no_gravity_on_ground && jump(&m).land_keeps_fraction);
    assert_eq!(jump(&m).land_depth, Some(5));
    assert_eq!(jump(&m).feet.as_ref().map(Vec::len), Some(2));
    let plain = Model::parse(SHAPED).unwrap();
    assert!(!jump(&plain).no_gravity_on_ground && !jump(&plain).land_keeps_fraction);
    assert!(!plain.to_toml().contains("no_gravity_on_ground"));
    assert!(!plain.to_toml().contains("land_keeps_fraction"));
    assert!(jump(&plain).land_depth.is_none());
}

/// Platformer 0.4.6's fall gravity kept from the last jump and a head
/// hit's hold parse, validate and round-trip, and are off by default; a
/// start without keeping is a problem.
#[test]
fn keep_fall_gravity_and_hold_of_0_4_6_parse_and_validate() {
    let with = |jump: &str| {
        shaped_edit(
            "[entities.hero.jump]\n",
            &format!("[entities.hero.jump]\n{jump}"),
        )
        .replace("platformer = \"0.4.0\"", "platformer = \"0.4.6\"")
    };
    let on = with(
        "keep_fall_gravity = true\nstart_fall_gravity = \"0.15625\"\n\
         head = { point = [8, 2], hits = [{ tile = \"solid\", speed = 1, becomes = \"empty\", hold = 14 }] }\n",
    );
    assert_eq!(problems(&on), Vec::<String>::new());
    let m = Model::parse(&on).unwrap();
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let jump = |m: &Model| m.entities["hero"].platformer.jump.clone().unwrap();
    assert!(jump(&m).keep_fall_gravity);
    assert_eq!(jump(&m).start_fall_gravity, Some(Fixed::from_raw(0x2800)));
    assert_eq!(jump(&m).head.unwrap().hits[0].hold, 14);
    let plain = Model::parse(SHAPED).unwrap();
    assert!(!jump(&plain).keep_fall_gravity);
    assert!(!plain.to_toml().contains("keep_fall_gravity"));
    let p = problems(&with("start_fall_gravity = \"0.25\"\n"));
    assert!(
        p.iter().any(|x| x.contains("only with keep_fall_gravity")),
        "{p:?}"
    );
    let p = problems(&with("keep_fall_gravity = true\nstart_fall_gravity = 0\n"));
    assert!(p.iter().any(|x| x.contains("above zero")), "{p:?}");
}

/// Platformer 0.4.11's actions that steer for the quick turn parse,
/// validate and round-trip, need `skid_turn_below` and name input actions
/// other than the directions.
#[test]
fn skid_turn_with_of_0_4_11_parses_and_validates() {
    let with = |run: &str| {
        shaped_edit(
            "[entities.hero.run]\n",
            &format!("[entities.hero.run]\n{run}"),
        )
        .replace("platformer = \"0.4.0\"", "platformer = \"0.4.11\"")
    };
    let on = with("skid_turn_below = \"0.6875\"\nskid_turn_with = [\"run\", \"down\"]\n");
    assert_eq!(problems(&on), Vec::<String>::new());
    let m = Model::parse(&on).unwrap();
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let run = m.entities["hero"].platformer.run.as_ref().unwrap();
    assert_eq!(run.skid_turn_with, ["run", "down"]);
    assert!(
        !Model::parse(SHAPED)
            .unwrap()
            .to_toml()
            .contains("skid_turn_with")
    );
    let p = problems(&with("skid_turn_with = [\"run\"]\n"));
    assert!(
        p.iter().any(|x| x.contains("needs skid_turn_below")),
        "{p:?}"
    );
    let p = problems(&with("skid_turn_below = 1\nskid_turn_with = [\"fire\"]\n"));
    assert!(
        p.iter().any(|x| x.contains("no input action `fire`")),
        "{p:?}"
    );
    let p = problems(&with("skid_turn_below = 1\nskid_turn_with = [\"left\"]\n"));
    assert!(p.iter().any(|x| x.contains("steer already")), "{p:?}");
}

/// Platformer 0.4.8's push mask and band of heights where tiles are met
/// parse, validate and round-trip, and are off by default.
#[test]
fn push_masks_and_collide_y_of_0_4_8_parse_and_validate() {
    let with = |jump: &str, run: &str| {
        shaped_edit(
            "[entities.hero.jump]\n",
            &format!("[entities.hero.jump]\n{jump}"),
        )
        .replacen(
            "[entities.hero.run]\n",
            &format!("[entities.hero.run]\n{run}"),
            1,
        )
        .replace("platformer = \"0.4.0\"", "platformer = \"0.4.8\"")
    };
    let on = with("collide_y = [0, 207]\n", "push_masks = true\n");
    assert_eq!(problems(&on), Vec::<String>::new());
    let m = Model::parse(&on).unwrap();
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let hero = &m.entities["hero"].platformer;
    assert_eq!(hero.jump.as_ref().unwrap().collide_y, Some([0, 207]));
    assert!(hero.run.as_ref().unwrap().push_masks);
    let plain = Model::parse(SHAPED).unwrap();
    assert!(
        plain.entities["hero"]
            .platformer
            .jump
            .as_ref()
            .unwrap()
            .collide_y
            .is_none()
    );
    assert!(!plain.to_toml().contains("push_masks"));
    let p = problems(&with("collide_y = [207, 0]\n", ""));
    assert!(p.iter().any(|x| x.contains("below the second")), "{p:?}");
    let hop: crate::platformer::Hop = toml::from_str("speed = -3\nland_first = true\n").unwrap();
    assert!(hop.land_first);
    let walk: crate::platformer::Walk =
        toml::from_str("speed = \"0.5\"\nwalls_at_top = true\n").unwrap();
    assert!(walk.walls_at_top);
    let plain: crate::platformer::Hop = toml::from_str("speed = -3\n").unwrap();
    assert!(!plain.land_first);
}

/// Core 0.4.12's part freezes, held counters, kept time, aligned
/// transitions and camera options, and platformer 0.4.9's touch every few
/// frames and pit event, parse, validate and round-trip; the core keys
/// need core 0.4.12, and bad names are refused.
#[test]
fn part_freezes_and_pit_events_of_0_4_12_parse_and_validate() {
    let model = |schema: &str, state: &str, extra: &str| {
        edit(
            "[[rules.game.states.dying.transition]]\n",
            &format!(
                "[rules.game.states.dying]\n{state}\n[[rules.game.states.dying.transition]]\n"
            ),
        )
        .replace("schema = \"0.4.0\"", &format!("schema = \"{schema}\""))
        .replace("platformer = \"0.4.0\"", "platformer = \"0.4.9\"")
            + extra
    };
    let state = "stops = [\"platformer.enemies\", \"counters\"]\nstops_at_once = [\"platformer.enemies\"]\nholds = [\"deaths\"]\nkeep_time = true\n";
    let on = model("0.4.12", state, "");
    assert_eq!(problems(&on), Vec::<String>::new());
    let m = Model::parse(&on).unwrap();
    let dying = &m.rules["game"].states["dying"];
    assert_eq!(dying.stops, ["platformer.enemies", "counters"]);
    assert!(dying.keep_time);
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    assert!(!has(&problems(&model("0.4.11", state, "")), "core schema"));
    let p = problems(&model(
        "0.4.12",
        "stops = [\"rules\", \"nothing\"]\nholds = [\"lives\"]\n",
        "",
    ));
    assert!(has(&p, "`rules` isn't a system"), "{p:?}");
    assert!(has(&p, "`nothing` isn't a system"), "{p:?}");
    assert!(has(&p, "no counter named `lives`"), "{p:?}");
    let p = problems(&model(
        "0.4.12",
        "stops_at_once = [\"counters\"]\nseen_every = 2\n",
        "",
    ));
    assert!(
        has(&p, "must be one of the systems the state stops"),
        "{p:?}"
    );
    assert!(has(&p, "only an entity's state has seen_every"), "{p:?}");
    let align = edit("after = 60\n", "after = 60\nalign = 4\n")
        .replace("schema = \"0.4.0\"", "schema = \"0.4.12\"");
    assert_eq!(problems(&align), Vec::<String>::new());
    assert_eq!(
        Model::parse(&align).unwrap().rules["game"].states["dying"].transition[0].align,
        Some(4)
    );
    let pit = edit(
        "kill_below = 208\n",
        "kill_below = 208\npit_event = \"hero.fell\"\n",
    )
    .replace("platformer = \"0.4.0\"", "platformer = \"0.4.9\"");
    assert!(has(
        &problems(&pit),
        "`hero.fell` isn't one of model.events"
    ));
    let pit = pit.replace("[input]", "events = [\"hero.fell\"]\n\n[input]");
    assert_eq!(problems(&pit), Vec::<String>::new());
    let m = Model::parse(&pit).unwrap();
    assert_eq!(
        m.levels["one"].platformer.pit_event.as_deref(),
        Some("hero.fell")
    );
    let touch = edit(
        "[entities.blob.patrol]\n",
        "[entities.blob.contact]\notherwise = \"hurt-player\"\nevery = 2\nphase = 1\n\n[entities.blob.patrol]\n",
    );
    assert_eq!(problems(&touch), Vec::<String>::new());
    let c = Model::parse(&touch).unwrap().entities["blob"]
        .platformer
        .contact
        .clone()
        .unwrap();
    assert_eq!((c.every, c.phase), (Some(2), 1));
    let bad = touch.replace("phase = 1", "phase = 2");
    assert!(has(&problems(&bad), "must be below `every`"));
}

/// Core 0.4.15's `enter_speed` is an entity state's and needs 0.4.15.
#[test]
fn enter_speed_of_0_4_15_is_an_entity_states() {
    let game = |schema: &str| {
        edit(
            "[[rules.game.states.dying.transition]]\n",
            "[rules.game.states.dying]\nenter_speed = [0, -4]\n[[rules.game.states.dying.transition]]\n",
        )
        .replace("schema = \"0.4.0\"", &format!("schema = \"{schema}\""))
    };
    assert!(has(
        &problems(&game("0.4.15")),
        "only an entity's state sets its speed"
    ));
    assert!(!has(&problems(&game("0.4.14")), "core schema"));
    let alone = edit(
        "[[rules.game.states.dying.transition]]\n",
        "[rules.game.states.dying]\nenter_keeps_fraction = true\n[[rules.game.states.dying.transition]]\n",
    )
    .replace("schema = \"0.4.0\"", "schema = \"0.4.15\"");
    assert!(has(&problems(&alone), "is for a state with `enter_speed`"));
}

/// Core 0.4.11's camera hold after a push and a spawn placed at the view's
/// edge parse, validate and round-trip, and need core 0.4.11.
#[test]
fn push_hold_at_edge_and_appear_moving_need_0_4_11() {
    let cam = |schema: &str| {
        shaped_edit(
            "scroll_back = false\n",
            "scroll_back = false\nby_move = { from = 80, full = 112, short_by = 1, push_hold = 16 }\n",
        )
        .replace("schema = \"0.4.0\"", &format!("schema = \"{schema}\""))
    };
    assert_eq!(problems(&cam("0.4.11")), Vec::<String>::new());
    let m = Model::parse(&cam("0.4.11")).unwrap();
    assert_eq!(m.cameras["main"].by_move.unwrap().push_hold, 16);
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    assert!(!has(&problems(&cam("0.4.10")), "core schema"));
    let spawn = |schema: &str| {
        edit(
            "params = { patrol_end = 140 }\n",
            "params = { patrol_end = 140 }\nat_edge = 24\nappear_moving = true\n",
        )
        .replace("schema = \"0.4.0\"", &format!("schema = \"{schema}\""))
    };
    assert_eq!(problems(&spawn("0.4.11")), Vec::<String>::new());
    let m = Model::parse(&spawn("0.4.11")).unwrap();
    let s = &m.levels["one"].core.spawn[1];
    assert_eq!(s.at_edge, Some(Fixed::from_int(24).unwrap()));
    assert!(s.appear_moving);
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    assert!(!has(&problems(&spawn("0.4.10")), "core schema"));
    let rest = |schema: &str, level: &str| {
        edit("kill_below = 208\n", &format!("kill_below = 208\n{level}"))
            .replace("schema = \"0.4.0\"", &format!("schema = \"{schema}\""))
    };
    let on = rest(
        "0.4.11",
        "spawn_ahead = 48\nspawn_slots = 5\nslot_rest = true\n",
    );
    assert_eq!(problems(&on), Vec::<String>::new());
    assert!(Model::parse(&on).unwrap().levels["one"].core.slot_rest);
    assert!(!has(
        &problems(&rest(
            "0.4.10",
            "spawn_ahead = 48\nspawn_slots = 5\nslot_rest = true\n"
        )),
        "core schema"
    ));
    assert!(has(
        &problems(&rest("0.4.11", "slot_rest = true\n")),
        "give spawn_slots"
    ));
}

/// Platformer 0.4.5's head point parses, validates and round-trips, and a
/// state's `set` changes it; its hits need cells that stop movement, whole
/// speeds and known events; core 0.4.6's block kinds need core 0.4.6.
#[test]
fn a_head_point_of_0_4_5_parses_and_validates() {
    let with = |head: &str, schema: &str| {
        shaped_edit(
            "[entities.hero.jump]\n",
            &format!("[entities.hero.jump]\nhead = {{ {head} }}\n"),
        )
        .replace("platformer = \"0.4.0\"", "platformer = \"0.4.5\"")
        .replace(
            "schema = \"0.4.0\"",
            &format!("schema = \"{schema}\"\nevents = [\"block.coin\"]"),
        )
    };
    let good = "point = [8, 18], above = 16, row_from = 4, keep_fraction = true, hits = [\
        { tile = \"solid\", speed = 1 }, \
        { tile = \"coin_block\", speed = 0, event = \"block.coin\", becomes = \"solid\" }]";
    let text = with(good, "0.4.6");
    assert_eq!(problems(&text), Vec::<String>::new());
    let m = Model::parse(&text).unwrap();
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let hd = m.entities["hero"]
        .platformer
        .jump
        .clone()
        .unwrap()
        .head
        .unwrap();
    assert_eq!(hd.hits[1].tile, crate::model::TileKind::CoinBlock);
    assert!(m.to_toml().contains("coin_block"));
    let bad = |head: &str, schema: &str, want: &str| {
        let p = problems(&with(head, schema));
        assert!(p.iter().any(|x| x.contains(want)), "{head}: {p:?}");
    };
    assert_eq!(problems(&with(good, "0.4.5")), Vec::<String>::new());
    bad("point = [8, \"1.5\"]", "0.4.6", "whole pixels");
    bad(
        "point = [8, 18], hits = [{ tile = \"ladder\", speed = 0 }]",
        "0.4.6",
        "stops movement",
    );
    bad(
        "point = [8, 18], hits = [{ tile = \"solid\", speed = \"0.5\" }]",
        "0.4.6",
        "must be whole",
    );
    bad(
        "point = [8, 18], hits = [{ tile = \"solid\", speed = 1, event = \"no.such\" }]",
        "0.4.6",
        "isn't an event",
    );
}

#[test]
fn a_springboard_of_0_4_4_parses_and_validates() {
    let spring = "[entities.spring]\nrole = \"interactive\"\nsize = [16, 16]\nhitbox = { x = 0, y = 0, w = 16, h = 16 }\n\n\
        [entities.spring.springboard]\nspeed = -7\nthrow_late = true\n\
        boost = { speed = -12, action = \"jump\", from_step = 1 }\n\
        steps = [{ frames = 4, move = 2, offset = 8 }, { frames = 1, move = -2 }]\n\n";
    let with = |s: &str| {
        edit("[levels.one]\n", &format!("{s}[levels.one]\n"))
            .replace("platformer = \"0.4.0\"", "platformer = \"0.4.4\"")
    };
    let text = with(spring);
    assert_eq!(problems(&text), Vec::<String>::new());
    let m = Model::parse(&text).unwrap();
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let sb = m.entities["spring"]
        .platformer
        .springboard
        .as_ref()
        .unwrap();
    assert_eq!(sb.steps.len(), 2);
    assert_eq!(
        sb.steps[1].offset,
        crate::Fixed::ZERO,
        "an offset defaults to 0"
    );
    assert_eq!(sb.boost.as_ref().unwrap().from_step, 1);
    assert!(!sb.boost.as_ref().unwrap().held);
    // Off by default: a model without one has none.
    assert!(
        small()
            .entities
            .values()
            .all(|e| e.platformer.springboard.is_none())
    );
    let bad = |from: &str, to: &str, want: &str| {
        let p = problems(&with(&spring.replace(from, to)));
        assert!(p.iter().any(|x| x.contains(want)), "{to}: {p:?}");
    };
    bad("speed = -7", "speed = 7", "a throw goes up");
    bad(
        "action = \"jump\"",
        "action = \"fly\"",
        "no input action `fly`",
    );
    bad(
        "from_step = 1",
        "from_step = 2",
        "must name one of the steps",
    );
    bad("frames = 4", "frames = 0", "at least 1 frame");
    bad(
        "[entities.spring.springboard]",
        "[entities.spring.fall]\ngravity = 1\nmax_fall = 4\n\n[entities.spring.springboard]",
        "leave out fall",
    );
}

#[test]
fn hud_sprites_and_counter_cycles_of_0_4_7_are_checked() {
    let present = r#"[assets.font]
kind = "font"
role = "font"

[assets.colors]
kind = "table"
role = "palettes"

[present.hud]
tiles = "tiles"
font = "font"
sprite = [{ at = [88, 25], tiles = "tiles", tile = 255, palette = 3, behind = true }]

[[present.cycles]]
palette = 3
index = 1
colors = "colors"
every = 8
counter = "deaths"
shift = -1

[evidence.present]
source = "observed"
confidence = 90
refs = [{ at = "notes.txt:1" }]

[entities.hero]"#;
    let with = |schema: &str| {
        edit("[entities.hero]", present)
            .replace("schema = \"0.4.0\"", &format!("schema = \"{schema}\""))
    };
    let good = with("0.4.7");
    assert_eq!(problems(&good), Vec::<String>::new());
    let m = Model::parse(&good).unwrap();
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let hud = m.present.as_ref().unwrap().hud.as_ref().unwrap();
    assert_eq!(hud.sprite[0].at, [88, 25]);
    assert!(hud.sprite[0].behind);
    let cycle = &m.present.as_ref().unwrap().cycles[0];
    assert_eq!(
        (cycle.counter.as_deref(), cycle.shift),
        (Some("deaths"), Some(-1))
    );
    let has = |p: &[String], want: &str| p.iter().any(|x| x.contains(want));
    let old = problems(&with("0.4.4"));
    assert!(!has(&old, "core schema"), "{old:?}");
    assert!(!has(&old, "core schema"), "{old:?}");
    let bad = problems(
        &good
            .replace("counter = \"deaths\"", "counter = \"nope\"")
            .replace("palette = 3, behind", "palette = 4, behind")
            .replace("palette = 3 }", "palette = 4 }"),
    );
    assert!(has(&bad, "no counter named `nope`"), "{bad:?}");
    assert!(has(&bad, "a sprite palette is 0 to 3"), "{bad:?}");
}

#[test]
fn a_counter_that_runs_when_frozen_needs_0_4_8_and_an_every_change() {
    let with = |schema: &str, change: &str| {
        edit(
            "[[counters.deaths.change]]\non = \"player.died\"",
            &format!(
                "[counters.deaths]\nruns_when_frozen = true\n\n[[counters.deaths.change]]\n{change}"
            ),
        )
        .replace("schema = \"0.4.0\"", &format!("schema = \"{schema}\""))
    };
    let good = with("0.4.8", "every = 8");
    assert_eq!(problems(&good), Vec::<String>::new());
    let m = Model::parse(&good).unwrap();
    assert!(m.counters["deaths"].runs_when_frozen);
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let old = problems(&with("0.4.7", "every = 8"));
    assert!(!has(&old, "core schema"), "{old:?}");
    let on = problems(&with("0.4.8", "on = \"player.died\""));
    assert!(has(&on, "needs an `every` change"), "{on:?}");
}

/// A clip counted by the body's timer (`clock = "body"`) needs core
/// 0.4.9, and takes `by_speed` as an `own` clip does.
#[test]
fn a_body_clock_clip_needs_0_4_9() {
    let clip = "sprite = { slot = \"tiles\", tiles = [0x10, 0x11, 0x12, 0x13], width = 2, palette = 0, clips = [{ clock = \"body\", frames = [{ tiles = [1, 2, 3, 4] }, { tiles = [5, 6, 7, 8] }], by_speed = [{ at_least = \"1\", every = 2 }, { every = 4 }] }] }";
    let text = edit(
        "sprite = { slot = \"tiles\", tiles = [0x10, 0x11, 0x12, 0x13], width = 2, palette = 0 }",
        clip,
    );
    let good = text.replacen("schema = \"0.4.0\"", "schema = \"0.4.9\"", 1);
    assert_eq!(problems(&good), Vec::<String>::new());
    let m = Model::parse(&good).unwrap();
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let c = &m.entities["hero"].core.sprite.as_ref().unwrap().clips[0];
    assert_eq!(c.clock, crate::model::Clock::Body);
    assert!(!has(&problems(&text), "core schema"));
}

#[test]
fn spawn_slots_order_draw_and_despawn_need_0_4_10_and_their_parts() {
    let with = |schema: &str, level: &str, spawn: &str, blob: &str| {
        edit("kill_below = 208\n", &format!("kill_below = 208\n{level}"))
            .replacen(
                "params = { patrol_end = 140 }\n",
                &format!("params = {{ patrol_end = 140 }}\n{spawn}"),
                1,
            )
            .replacen(
                "[entities.blob]\nrole = \"enemy\"\n",
                &format!("[entities.blob]\nrole = \"enemy\"\n{blob}"),
                1,
            )
            .replace("schema = \"0.4.0\"", &format!("schema = \"{schema}\""))
    };
    let level = "spawn_ahead = 48\ndespawn_behind = 72\ndespawn_at = \"position\"\nspawn_slots = 5\nspawn_in_order = true\nappear_still = true\ndraw = { ahead = 128, step = 32, every = 4, pause = 1 }\n";
    let good = with(
        "0.4.10",
        level,
        "drawn = true\n",
        "no_slot = true\ndespawn_behind = 16\n",
    );
    assert_eq!(problems(&good), Vec::<String>::new());
    let m = Model::parse(&good).unwrap();
    let l = &m.levels["one"].core;
    assert_eq!(l.spawn_slots, Some(5));
    assert!(l.spawn_in_order && l.appear_still && l.spawn[1].drawn);
    assert_eq!(l.despawn_at, Some(crate::model::DespawnAt::Position));
    assert_eq!(
        l.draw.as_ref().map(|d| (d.step, d.every, d.pause)),
        Some((32, 4, 1))
    );
    assert!(m.entities["blob"].core.no_slot);
    assert_eq!(Model::parse(&m.to_toml()).unwrap(), m);
    let old = problems(&with("0.4.9", level, "", ""));
    assert!(!has(&old, "core schema"), "{old:?}");
    let old = problems(&with("0.4.9", "", "", "no_slot = true\n"));
    assert!(!has(&old, "core schema"), "{old:?}");
    let bare = problems(&with("0.4.10", "spawn_slots = 5\n", "drawn = true\n", ""));
    assert!(has(&bare, "give spawn_ahead"), "{bare:?}");
    assert!(has(&bare, "give the level a draw"), "{bare:?}");
    let step = problems(&with(
        "0.4.10",
        "spawn_ahead = 48\ndraw = { ahead = 128, step = 24, every = 0 }\n",
        "",
        "",
    ));
    assert!(has(&step, "a whole number of tiles"), "{step:?}");
    assert!(has(&step, "levels.one.draw.every"), "{step:?}");
    let at = problems(&with("0.4.10", "despawn_at = \"position\"\n", "", ""));
    assert!(has(&at, "give despawn_behind or despawn_ahead"), "{at:?}");
}

#[test]
fn a_model_listing_an_unknown_feature_is_refused_by_name() {
    // The unknown feature comes with a field this build doesn't know
    // either: the refusal names the feature, not the field.
    let text = SMALL
        .replacen(
            "extensions = { platformer = \"0.4.0\" }",
            "extensions = { platformer = \"0.4.0\" }\nfeatures = [\"rope.swing\"]",
            1,
        )
        .replacen("[input]", "[rope]\nswing = 1\n\n[input]", 1);
    match Model::parse(&text) {
        Err(crate::Error::Features { names, message }) => {
            assert_eq!(names, ["rope.swing"]);
            assert!(message.contains("`rope.swing`"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_model_listing_known_features_is_accepted_and_must_list_what_it_uses() {
    let opt = "tile_collision = true\nno_slot = true\n";
    let with = |list: &str, cam: &str| {
        SMALL
            .replacen(
                "extensions = { platformer = \"0.4.0\" }",
                &format!("extensions = {{ platformer = \"0.4.0\" }}\nfeatures = [{list}]"),
                1,
            )
            .replacen("tile_collision = true\n", cam, 1)
    };
    let cam = opt;
    let base = SMALL.replacen("tile_collision = true\n", opt, 1);
    let used = Model::parse(&base).unwrap().features_used();
    assert!(used.contains("entity.no_slot"), "{used:?}");
    assert!(!used.is_empty());
    let all: Vec<String> = used.iter().map(|f| format!("\"{f}\"")).collect();
    let m = Model::parse(&with(&all.join(", "), cam)).unwrap();
    let p = validate(&m);
    assert!(
        !p.iter().any(|x| x.path.starts_with("model.features")),
        "{p:?}"
    );
    // A list that leaves out a used feature is a problem, and so is a
    // platformer feature in a model without the extension.
    let m = Model::parse(&with("\"sprite.cells\"", cam)).unwrap();
    assert!(
        validate(&m)
            .iter()
            .any(|x| x.path == "model.features" && x.message.contains("doesn't list")),
        "{:?}",
        validate(&m)
    );
    // An older patch in `schema` is read as the same version.
    assert!(crate::validate::version_supported(
        "0.4.23",
        crate::CORE_VERSION
    ));
    assert!(crate::validate::version_supported(
        "0.4",
        crate::CORE_VERSION
    ));
    assert!(!crate::validate::version_supported(
        "0.5",
        crate::CORE_VERSION
    ));
}

/// A small top-down model: eight-way movement, two classes, a class
/// select, and a camera on both axes.
pub(crate) const TOPDOWN: &str = r#"
[model]
schema = "0.4.0"
name = "maze"
title = "Maze"
view = "top-down"
frame_rate = "60"
extensions = { topdown = "0.1" }
systems = ["rules", "entity_states", "counters", "spawn", "topdown.player", "exits", "camera"]

[input]
device = "nes-pad"
actions = { left = "left", right = "right", up = "up", down = "down", next = "select", pick = "a" }

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

[entities.warrior]
role = "player"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
tile_collision = true
move8 = { speed = 1 }
class = { name = "Warrior", armor = 2 }

[entities.elf]
role = "player"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
tile_collision = true
move8 = { speed = "1.5" }
class = { name = "Elf" }

[levels.one]
grid = "map"
tileset = "blocks"
solid_property = "solid"
tile_size = 16
size = [16, 16]
class_select = { classes = ["warrior", "elf"], next = "next", pick = "pick" }

[[levels.one.spawn]]
entity = "warrior"
x = 32
y = 32

[cameras.main]
follow = "warrior"
axis = "both"
size = [256, 240]
lead = [120, 112]
clamp_to_level = true

[evidence.assets]
source = "stated"
confidence = 100
refs = [{ at = "notes.txt:1" }]

[evidence.entities]
source = "stated"
confidence = 100
refs = [{ at = "notes.txt:1" }]

[evidence.levels]
source = "stated"
confidence = 100
refs = [{ at = "notes.txt:1" }]

[evidence.input]
source = "stated"
confidence = 100
refs = [{ at = "notes.txt:1" }]

[evidence.cameras]
source = "stated"
confidence = 100
refs = [{ at = "notes.txt:1" }]
"#;

#[test]
fn a_topdown_model_is_valid_and_round_trips() {
    let p = problems(TOPDOWN);
    assert_eq!(p, Vec::<String>::new());
    let m = Model::parse(TOPDOWN).unwrap();
    let w = &m.entities["warrior"];
    assert_eq!(
        w.topdown.move8.as_ref().unwrap().speed,
        Fixed::from_int(1).unwrap()
    );
    assert_eq!(w.topdown.class.as_ref().unwrap().name, "Warrior");
    assert!(w.platformer.is_empty());
    let sel = m.levels["one"].topdown.class_select.as_ref().unwrap();
    assert_eq!(sel.classes, ["warrior", "elf"]);
    // Written and read again, the same model.
    let again = Model::parse(&toml::to_string(&m).unwrap()).unwrap();
    assert_eq!(again, m);
}

#[test]
fn topdown_parts_are_checked() {
    let td = |from: &str, to: &str| {
        assert_eq!(TOPDOWN.matches(from).count(), 1, "{from}");
        problems(&TOPDOWN.replace(from, to))
    };
    // Its parts need the extension declared, and its system in the order.
    let p = td("extensions = { topdown = \"0.1\" }", "extensions = {}");
    assert!(has(&p, "topdown"), "{p:?}");
    // move8's actions must be input actions.
    let p = td(", next = \"select\"", "");
    assert!(has(&p, "no input action `next`"), "{p:?}");
    let p = td("up = \"up\", ", "");
    assert!(has(&p, "`up`"), "{p:?}");
    // A class select names entities with a class.
    let p = td(
        "classes = [\"warrior\", \"elf\"]",
        "classes = [\"warrior\", \"ghost\"]",
    );
    assert!(has(&p, "`ghost` isn't an entity with a `class`"), "{p:?}");
    // The speed is more than 0.
    let p = td("move8 = { speed = 1 }", "move8 = { speed = 0 }");
    assert!(has(&p, "more than 0"), "{p:?}");
}

#[test]
fn no_extension_reuses_another_extensions_names() {
    let exts = crate::validate::EXTENSIONS;
    for (i, a) in exts.iter().enumerate() {
        for b in &exts[i + 1..] {
            for c in a.entity_components {
                assert!(!b.entity_components.contains(c), "{c}");
            }
            for f in a.level_fields {
                assert!(!b.level_fields.contains(f), "{f}");
            }
            for s in a.systems {
                assert!(!b.systems.contains(s), "{s}");
            }
        }
    }
}
