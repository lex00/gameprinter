//! The corpus's derivation, coverage and order on the templates' models,
//! without a ROM: the cells a model gives, that the same inputs give the
//! same cells, that a model refined or with a mechanic added keeps every
//! cell it had, and the minimal set and queue order of a made-up
//! excitation.

use std::collections::BTreeMap;
use std::path::Path;

use super::coverage::{self, Seen};
use super::derive::{self, Facts};
use super::{Anchor, Cell, Clip, Status};
use crate::mapping::Mapping;
use crate::mechanics;

fn template(name: &str) -> (String, String) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates")
        .join(name);
    (
        std::fs::read_to_string(dir.join("model.toml")).unwrap(),
        std::fs::read_to_string(dir.join("mapping.toml")).unwrap(),
    )
}

fn derived(
    model: &str,
    mapping: &str,
    anchors: &BTreeMap<String, Anchor>,
) -> (Facts, Vec<super::Context>, Vec<Cell>) {
    let m = gp_model::Model::parse(model).unwrap();
    let cat = mechanics::catalog(&m, &gp_sim::Exts::new());
    let map = Mapping::parse(mapping).unwrap();
    derive::derive(&m, &cat, &map, anchors).unwrap()
}

fn anchor(frames: u64) -> Anchor {
    Anchor {
        frames,
        from: "ground.still/s1".into(),
        held: String::new(),
        says: String::new(),
    }
}

fn ids(cells: &[Cell]) -> Vec<String> {
    cells.iter().map(|c| c.id.clone()).collect()
}

#[test]
fn moss_hop_gives_the_same_cells_every_time_named_by_mechanic_pattern_and_context() {
    let (model, mapping) = template("mosshop");
    let a = BTreeMap::from([(derive::TOP.to_owned(), anchor(20))]);
    let (_, cx1, c1) = derived(&model, &mapping, &a);
    let (_, cx2, c2) = derived(&model, &mapping, &a);
    assert_eq!(
        serde_json::to_vec(&c1).unwrap(),
        serde_json::to_vec(&c2).unwrap()
    );
    assert_eq!(cx1, cx2);
    let all = ids(&c1);
    for want in [
        "core.idle:idle-60:ground.still",
        "platformer.jump:jump.tap:ground.still",
        "platformer.jump:jump.hold-16:ground.still",
        "platformer.run:fwd+run.hold-64:ground.still",
        "platformer.run:back.doublet-32:ground.walk_top",
        "platformer.contact:jump+fwd.stomp-8:enemy.near[slug]",
        "check.mapping:y-by-vy.check:ground.still",
    ] {
        assert!(all.iter().any(|i| i == want), "{want} not in {all:#?}");
    }
    // Moss Hop's jump has no bands, and its mapping no vx: no band
    // contexts and no x-by-vx check.
    assert!(!cx1.iter().any(|c| c.id.starts_with("ground.band")));
    assert!(!all.iter().any(|i| i.contains("x-by-vx")));
    // Held past the top of the rise (20 frames here) is dominated; B alone
    // has no effect by the record, but one tap stays as a check.
    let get = |id: &str| c1.iter().find(|c| c.id == id).unwrap();
    assert_eq!(
        get("platformer.jump:jump.hold-32:ground.still").status,
        Status::Pruned
    );
    assert_eq!(
        get("platformer.jump:jump.hold-16:ground.still").status,
        Status::Active
    );
    assert_eq!(
        get("platformer.run:run.tap:ground.still").status,
        Status::Check
    );
    assert_eq!(
        get("platformer.run:run.hold-64:ground.still").status,
        Status::Pruned
    );
    // A second jump in the air checks that nothing happens.
    assert_eq!(
        get("platformer.jump:jump.again:air.rising").status,
        Status::Check
    );
    // No ID holds a frame number but the fixed holds' and seeds'.
    assert!(!all.iter().any(|i| i.contains("-20")), "{all:#?}");
}

#[test]
fn refining_a_number_or_remeasuring_an_anchor_renames_nothing() {
    let (model, mapping) = template("mosshop");
    let a = BTreeMap::from([(derive::TOP.to_owned(), anchor(20))]);
    let (_, _, before) = derived(&model, &mapping, &a);
    let refined = model.replace("gravity = \"0.25\"", "gravity = \"0.1875\"");
    assert_ne!(refined, model);
    let (_, _, after) = derived(&refined, &mapping, &a);
    assert_eq!(ids(&before), ids(&after));
    assert_eq!(before, after);
    // A top measured at 21 frames moves the hold to the top, not its ID.
    let b = BTreeMap::from([(derive::TOP.to_owned(), anchor(21))]);
    let (_, _, again) = derived(&model, &mapping, &b);
    assert_eq!(ids(&before), ids(&again));
}

#[test]
fn adding_a_mechanic_to_a_copy_only_adds_cells() {
    let (model, mapping) = template("mosshop");
    let a = BTreeMap::from([(derive::TOP.to_owned(), anchor(20))]);
    let (_, cx, before) = derived(&model, &mapping, &a);
    let more = model
        .replace("[input.actions]\n", "[input.actions]\ndown = \"down\"\nup = \"up\"\n")
        .replace(
            "[entities.player.run]",
            "[entities.player.climb]\ndown = \"down\"\nspeed = 1\nup = \"up\"\n\n[entities.player.run]",
        );
    assert_ne!(more, model);
    let (f, cx2, after) = derived(&more, &mapping, &a);
    assert!(f.climb, "the copy climbs");
    assert!(cx2.iter().any(|c| c.id == "ladder"));
    for c in &before {
        let same = after.iter().find(|x| x.id == c.id);
        assert_eq!(same, Some(c), "{} changed", c.id);
    }
    for c in &cx {
        assert!(cx2.contains(c), "{} changed", c.id);
    }
    assert!(after.len() > before.len());
}

fn clip(id: &str, cell: &str, excites: &[&str]) -> Clip {
    Clip {
        id: id.into(),
        cell: cell.into(),
        start: "ground.still/s1".into(),
        presses: Vec::new(),
        frames: 60,
        spec: String::new(),
        status: Status::Active,
        original: String::new(),
        excites: excites.iter().map(|s| (*s).to_owned()).collect(),
        enters: Vec::new(),
        effect: Vec::new(),
        minimal: false,
    }
}

fn cell(id: &str, pattern: &str, context: &str, mechanic: &str) -> Cell {
    Cell {
        id: id.into(),
        mechanic: mechanic.into(),
        pattern: super::Pattern {
            name: pattern.into(),
            action: String::new(),
            with: Vec::new(),
            frames: None,
            anchor: None,
            event: None,
        },
        context: context.into(),
        tier: 1,
        name: id.into(),
        presses: Vec::new(),
        frames: 60,
        status: Status::Active,
        reason: None,
        requires: Vec::new(),
        excites: Vec::new(),
        enters: Vec::new(),
        check: None,
    }
}

#[test]
fn the_minimal_set_covers_each_number_twice_and_separates_a_pair_that_moves_together() {
    let cells = vec![
        cell("hold16", "hold", "ground.still", "m"),
        cell("hold32", "hold", "ground.still", "m"),
        cell("tap", "tap", "ground.still", "m"),
        cell("edge", "hold", "ground.edge[0].over", "m"),
        cell("chk", "check", "ground.still", "check.mapping"),
    ];
    let clips = vec![
        clip("hold16@s1", "hold16", &["speed", "gravity"]),
        clip("hold32@s1", "hold32", &["speed", "gravity"]),
        clip("tap@s1", "tap", &["speed", "gravity"]),
        clip("edge@s1", "edge", &["below"]),
        clip("chk@s1", "chk", &[]),
    ];
    let params: Vec<(String, String)> = ["speed", "gravity", "below", "max_fall"]
        .iter()
        .map(|p| ((*p).to_owned(), "m".to_owned()))
        .collect();
    let cov = coverage::compute(&params, &cells, &clips, &[]);
    let by = |p: &str| cov.params.iter().find(|x| x.path == p).unwrap();
    // Two holds from one context are one family: speed has two
    // independent clips (a hold and the tap), not three.
    assert_eq!(by("speed").independent, 2);
    assert!(by("speed").covered);
    assert!(!by("below").covered);
    assert!(by("max_fall").clips.is_empty());
    assert_eq!(cov.minimal, ["hold16@s1", "tap@s1", "edge@s1"]);
    // Speed and gravity always move together: a pair no clip separates.
    let pair = cov
        .pairs
        .iter()
        .find(|p| p.a == "speed" && p.b == "gravity")
        .unwrap();
    assert!(pair.separated_by.is_empty());
    let kinds: Vec<(&str, &str)> = cov
        .gaps
        .iter()
        .map(|g| (g.kind.as_str(), g.what.as_str()))
        .collect();
    assert!(kinds.contains(&("param", "below")));
    assert!(kinds.contains(&("param", "max_fall")));
    assert!(kinds.contains(&("pair", "speed gravity")));
    assert!(cov.says.starts_with("2 of 4 numbers"), "{}", cov.says);
}

#[test]
fn the_queue_takes_checks_then_starred_then_the_minimal_set_then_the_thinnest() {
    let cells = vec![
        cell("a", "hold", "ground.still", "m"),
        cell("b", "tap", "ground.still", "m"),
        cell("c", "hold", "air.rising", "m"),
        cell("d", "hold", "water", "probe.swim"),
        cell("chk", "check", "ground.still", "check.mapping"),
    ];
    let mut clips = vec![
        clip("a@s1", "a", &["speed"]),
        clip("b@s1", "b", &["gravity"]),
        clip("c@s1", "c", &["gravity"]),
        clip("d@s1", "d", &[]),
        clip("chk@s1", "chk", &[]),
        clip("skipped@s1", "a", &["speed"]),
    ];
    clips[1].minimal = true;
    clips[3].status = Status::Probe;
    let marks = BTreeMap::from([
        ("c@s1".to_owned(), "starred".to_owned()),
        ("skipped@s1".to_owned(), "skipped".to_owned()),
    ]);
    let order = coverage::rank(&clips, &cells, &marks, &BTreeMap::new());
    assert_eq!(order, ["chk@s1", "c@s1", "b@s1", "a@s1", "d@s1"]);
    // Among equals, the number fewest passing clips move comes first, then
    // the earliest first wrong frame.
    let plain = vec![
        clip("x@s1", "a", &["speed"]),
        clip("y@s1", "a", &["gravity"]),
        clip("z@s1", "a", &["gravity"]),
        clip("w@s1", "a", &["speed"]),
    ];
    let seen = BTreeMap::from([
        (
            "w@s1".to_owned(),
            Seen {
                passing: true,
                first_wrong: None,
            },
        ),
        (
            "y@s1".to_owned(),
            Seen {
                passing: false,
                first_wrong: Some(30),
            },
        ),
        (
            "z@s1".to_owned(),
            Seen {
                passing: false,
                first_wrong: Some(4),
            },
        ),
    ]);
    let order = coverage::rank(&plain, &cells, &BTreeMap::new(), &seen);
    assert_eq!(order, ["z@s1", "y@s1", "x@s1", "w@s1"]);
}

#[test]
fn two_clips_that_move_a_pair_in_another_ratio_tell_it_apart() {
    let cells = vec![
        cell("tap", "tap", "ground.band[1]", "m"),
        cell("hold", "hold", "ground.band[1]", "m"),
    ];
    let mut tap = clip("tap@s1", "tap", &["speed", "gravity_held"]);
    tap.effect = vec![40, 2];
    let mut hold = clip("hold@s1", "hold", &["speed", "gravity_held"]);
    hold.effect = vec![40, 30];
    let params: Vec<(String, String)> = ["speed", "gravity_held"]
        .iter()
        .map(|p| ((*p).to_owned(), "m".to_owned()))
        .collect();
    let cov = coverage::compute(&params, &cells, &[tap.clone(), hold.clone()], &[]);
    assert_eq!(cov.pairs[0].separated_by, ["tap@s1", "hold@s1"]);
    assert!(cov.gaps.iter().all(|g| g.kind != "pair"), "{:?}", cov.gaps);
    assert_eq!(cov.minimal, ["hold@s1", "tap@s1"]);
    // The same ratio tells nothing apart.
    hold.effect = vec![60, 3];
    let cov = coverage::compute(&params, &cells, &[tap, hold], &[]);
    assert!(cov.pairs[0].separated_by.is_empty());
}
