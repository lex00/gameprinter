//! `assets_edit` on the made-up game (crate::testgame): ops placed with
//! assets_place's checks, the history and undo, the recorded-evidence gate.
//! No game data and no `gp` binary.

use std::fs;

use serde_json::{Value, json};

use crate::{Config, catalog, tools};

const HERO_EVIDENCE: &str = r#"
[evidence."entities.hero"]
source = "stated"
confidence = 100
note = "the test's own hero"
"#;

struct Fixture {
    _dir: tempfile::TempDir,
    config: Config,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (config, _) = crate::testgame::config(dir.path());
        // The made-up model has no evidence; the hero's sprite gets some, so
        // the model knows it well enough to edit.
        let model = config.work.join("copy/model.toml");
        let mut text = fs::read_to_string(&model).unwrap();
        text.push_str(HERO_EVIDENCE);
        fs::write(&model, text).unwrap();
        Self { _dir: dir, config }
    }

    fn call(&self, args: Value) -> (Value, bool) {
        let r = tools::call(&self.config, "assets_edit", args).expect("a listed tool");
        (r.json, r.is_error)
    }

    fn sheet(&self) -> Vec<u8> {
        fs::read(self.config.work.join("copy/assets/tiles.png")).unwrap()
    }

    fn slot(&self) -> Value {
        let s = catalog::State::read(&self.config.work.join("copy/assets"))
            .unwrap()
            .unwrap();
        serde_json::to_value(&s.slots["tiles"]).unwrap()
    }
}

/// Erase the top two rows of the hero's solid sprite, then outline what's
/// left in index 3: row 1 becomes the outline.
fn edit_ops() -> Value {
    json!([
        { "op": "fill", "region": { "x": 0, "y": 0, "w": 16, "h": 2 }, "index": 0 },
        { "op": "outline", "index": 3 },
    ])
}

#[test]
fn an_edit_is_placed_previewed_and_undone() {
    let f = Fixture::new();
    let (r, err) = f.call(json!({
        "copy": "copy", "entity": "hero", "ops": edit_ops(), "prompt": "a darker cap",
    }));
    assert!(!err, "{r}");
    assert_eq!(r["written"], true, "{r}");
    assert_eq!(r["simulation"]["unchanged"], true);
    assert_eq!(r["tiles_changed"], json!([16, 17]), "{r}");
    assert_eq!(r["ops"][0]["changed"], 32);
    assert_eq!(r["ops"][1]["changed"], 16);
    assert_eq!(r["after"][1], "3333333333333333");
    assert_eq!(r["history"]["seq"], 1);
    // The recorded baseline shows the hero, so the edit had evidence.
    assert!(r["contexts"]["seen"].as_u64().unwrap() > 0, "{r}");
    assert_eq!(r["readiness"]["ready"], true, "{r}");
    assert!(r["contexts"]["poses"][0]["pose"].is_string(), "{r}");
    assert!(r["preview"]["png_base64"].as_str().unwrap().len() > 100);
    assert!(
        f.config.work.join("copy/previews/edit-1.png").is_file(),
        "the preview is kept beside the history"
    );
    let slot = f.slot();
    assert_eq!(slot["status"], "replaced");
    assert!(slot["generator"].as_str().unwrap().contains("fill,outline"));
    assert_eq!(slot["note"], "a darker cap");

    let edited = f.sheet();
    let (u, err) = f.call(json!({ "copy": "copy", "undo": true }));
    assert!(!err, "{u}");
    assert_eq!(u["undone"]["seq"], 1);
    assert_eq!(u["history"]["depth"], 0);
    assert_ne!(f.sheet(), edited);
    // The slot is the original again, status and license included.
    let slot = f.slot();
    assert_eq!(slot["status"], "original", "{slot}");
    assert_eq!(slot["origin"], "source");
    let (u, err) = f.call(json!({ "copy": "copy", "undo": true }));
    assert!(
        err && u["error"].as_str().unwrap().contains("no edit"),
        "{u}"
    );
}

#[test]
fn a_dry_run_and_a_bad_op_write_nothing() {
    let f = Fixture::new();
    let (r, err) = f.call(json!({
        "copy": "copy", "entity": "hero", "ops": edit_ops(), "place": false,
    }));
    assert!(!err, "{r}");
    assert_eq!(r["written"], false);
    assert!(r["critic"]["score"].is_number(), "{r}");
    assert!(!f.config.work.join("copy/assets").exists());

    let (r, err) = f.call(json!({
        "copy": "copy", "entity": "hero",
        "ops": [{ "op": "recolor", "pairs": [[1, 4]] }],
    }));
    assert!(err, "{r}");
    assert!(
        r["error"].as_str().unwrap().contains("op 0 (recolor)"),
        "{r}"
    );
    assert!(!f.config.work.join("copy/assets").exists());
}

#[test]
fn undo_refuses_when_the_sprite_changed_since() {
    let f = Fixture::new();
    let (r, err) = f.call(json!({ "copy": "copy", "entity": "hero", "ops": edit_ops() }));
    assert!(!err, "{r}");
    // Another tool writes the hero's sprite.
    let r = tools::call(
        &f.config,
        "assets_generate",
        json!({ "copy": "copy", "slot": "tiles", "entity": "hero", "seed": 3 }),
    )
    .unwrap();
    assert!(!r.is_error, "{}", r.json);
    let (u, err) = f.call(json!({ "copy": "copy", "undo": true }));
    assert!(err, "{u}");
    assert!(
        u["error"].as_str().unwrap().contains("changed since"),
        "{u}"
    );
}

#[test]
fn a_sprite_the_model_doesnt_know_is_refused_with_what_is_missing() {
    let f = Fixture::new();
    // The buddy has a sprite but no spawn, no baseline shows it and no
    // evidence backs it.
    let (r, err) = f.call(json!({
        "copy": "copy", "entity": "buddy", "ops": [{ "op": "flip", "axis": "x" }],
    }));
    assert!(err, "{r}");
    assert_eq!(r["stage"], "model");
    let missing: Vec<&str> = r["readiness"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["status"] == "missing")
        .map(|i| i["item"].as_str().unwrap())
        .collect();
    assert_eq!(missing, ["drawn", "evidence"], "{r}");
    assert!(r["error"].as_str().unwrap().contains("gp capture"), "{r}");
    assert!(!f.config.work.join("copy/assets").exists());
}

#[test]
fn an_exported_sprite_comes_back_through_the_edit_checks() {
    let f = Fixture::new();
    let r = tools::call(
        &f.config,
        "assets_export",
        json!({ "copy": "copy", "entity": "hero" }),
    )
    .unwrap();
    assert!(!r.is_error, "{}", r.json);
    let png = std::path::PathBuf::from(r.json["image"].as_str().unwrap());
    let side: Value =
        serde_json::from_str(&fs::read_to_string(r.json["sidecar"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(side["image_size"], json!([16, 16]));
    assert_eq!(side["tiles"], json!([16, 17, 18, 19]));
    assert!(
        fs::read_to_string(r.json["palette_file"].as_str().unwrap())
            .unwrap()
            .starts_with("GIMP Palette")
    );

    // Loading the file as exported changes nothing.
    let load = json!([{ "op": "load", "file": png }]);
    let (r, err) = f.call(json!({ "copy": "copy", "entity": "hero", "ops": load }));
    assert!(!err, "{r}");
    assert_eq!(r["pixels_changed"], 0, "{r}");

    // The person draws in index 2 and saves: it's placed like any edit.
    let colors = [[0u8; 3], [1, 1, 1], [2, 2, 2], [3, 3, 3]];
    let mut pic = crate::exchange::read_image(&png, (16, 16), {
        // Read back with the file's own palette.
        let d = png::Decoder::new(std::io::Cursor::new(fs::read(&png).unwrap()));
        let r = d.read_info().unwrap();
        let p = r.info().palette.clone().unwrap();
        let mut c = colors;
        for (i, rgb) in p.chunks(3).take(4).enumerate() {
            c[i] = [rgb[0], rgb[1], rgb[2]];
        }
        c
    })
    .unwrap();
    pic.px[17] = 2;
    fs::write(&png, crate::exchange::write_indexed(&pic, colors)).unwrap();
    let (r, err) = f.call(json!({ "copy": "copy", "entity": "hero", "ops": load }));
    assert!(!err, "{r}");
    assert_eq!(r["written"], true, "{r}");
    assert_eq!(r["pixels_changed"], 1);
    assert_eq!(r["history"]["seq"], 1);
}

/// The made-up hero's sprite as rows, from the copy's sheet.
fn hero_rows(f: &Fixture) -> Vec<String> {
    let t = crate::place::target(&f.config, "copy", "tiles", Some("hero")).unwrap();
    crate::asset_edit::sprite_pic(&t).unwrap().rows()
}

#[test]
fn a_proposal_is_placed_by_accept_and_dropped_by_reject() {
    let f = Fixture::new();
    let original = hero_rows(&f);
    let (p, err) = f.call(json!({
        "copy": "copy", "entity": "hero", "ops": edit_ops(), "place": false, "prompt": "a cap",
    }));
    assert!(!err, "{p}");
    assert_eq!(p["status"], "proposed", "{p}");
    assert_eq!(p["written"], false);
    assert_eq!(p["item"], "copy:1");
    // The in-context preview drew the edit where the baseline shows it.
    assert!(
        p["in_context"]["shots"]
            .as_array()
            .is_some_and(|s| !s.is_empty()),
        "{p}"
    );
    assert!(p["in_context"]["png_base64"].is_string());
    assert_eq!(
        p["in_context"]["readability"]["name"],
        "context.readability"
    );
    assert!(
        f.config
            .work
            .join("copy/previews/edit-1-context.png")
            .is_file()
    );
    assert!(
        !f.config.work.join("copy/assets").exists(),
        "a proposal writes no art"
    );
    let (h, _) = (
        tools::call(&f.config, "assets_history", json!({ "copy": "copy" }))
            .unwrap()
            .json,
        (),
    );
    assert_eq!(h["waiting"], json!([1]), "{h}");

    let (a, err) = f.call(json!({ "copy": "copy", "accept": 1, "author": "person" }));
    assert!(!err, "{a}");
    assert_eq!(a["status"], "accepted", "{a}");
    assert_eq!(a["written"], true);
    assert_eq!(hero_rows(&f)[1], "3333333333333333");
    // Accepting it again changes nothing (a double click in the panel).
    let (again, err) = f.call(json!({ "copy": "copy", "accept": 1 }));
    assert!(!err && again["written"] == false, "{again}");

    // A second proposal, rejected: nothing changes.
    let edited = f.sheet();
    let (p2, _) = f.call(json!({
        "copy": "copy", "entity": "hero", "ops": [{ "op": "flip", "axis": "y" }], "place": false,
    }));
    assert_eq!(p2["seq"], 2, "{p2}");
    let (r, err) = f.call(json!({ "copy": "copy", "reject": 2 }));
    assert!(!err, "{r}");
    assert_eq!(r["status"], "rejected");
    assert_eq!(f.sheet(), edited);
    let h = tools::call(&f.config, "assets_history", json!({ "copy": "copy" }))
        .unwrap()
        .json;
    assert_eq!(h["placed"][0]["status"], "accepted", "{h}");
    assert_eq!(h["placed"][0]["decided_by"], "person");
    assert_eq!(h["proposals"][1]["status"], "rejected");

    // Rejecting a placed edit undoes it.
    let (r, err) = f.call(json!({ "copy": "copy", "reject": 1 }));
    assert!(!err, "{r}");
    assert_eq!(hero_rows(&f), original);
}

#[test]
fn undo_goes_back_many_steps_and_redo_returns() {
    let f = Fixture::new();
    let first = hero_rows(&f);
    let (r1, _) = f.call(json!({ "copy": "copy", "entity": "hero", "ops": edit_ops() }));
    assert_eq!(r1["written"], true, "{r1}");
    let second = hero_rows(&f);
    let (r2, _) = f.call(json!({ "copy": "copy", "entity": "hero", "ops": [{ "op": "pixels", "set": [[0, 15, 0]] }] }));
    assert_eq!(r2["written"], true, "{r2}");
    assert_ne!(hero_rows(&f), second);
    // Edit 1 changed tile 16 and so did edit 2 (pixel 0,15 is in tile 18).
    let (u, err) = f.call(json!({ "copy": "copy", "undo": true }));
    assert!(!err, "{u}");
    assert_eq!(hero_rows(&f), second);
    let (u, err) = f.call(json!({ "copy": "copy", "undo": true }));
    assert!(!err, "{u}");
    assert_eq!(hero_rows(&f), first);
    assert_eq!(
        f.slot()["status"],
        "original",
        "two undos put the slot back as it was"
    );
    assert!(
        f.call(json!({ "copy": "copy", "undo": true })).1,
        "nothing left"
    );
    let (r, err) = f.call(json!({ "copy": "copy", "redo": true }));
    assert!(!err, "{r}");
    assert_eq!(hero_rows(&f), second);
    let (r, err) = f.call(json!({ "copy": "copy", "redo": true }));
    assert!(!err, "{r}");
    assert_eq!(r["redone"]["seq"], 2, "{r}");
    // Edit 1 changed tiles 16 and 17, edit 2 tile 18: edit 1 can be
    // undone alone, leaving edit 2.
    let (r, err) = f.call(json!({ "copy": "copy", "undo": true, "seq": 1 }));
    assert!(!err, "{r}");
    let mut want = first.clone();
    want[15].replace_range(0..1, "0");
    assert_eq!(hero_rows(&f), want);
    // A later edit over tile 18 keeps edit 2 from being undone alone.
    let (r3, _) = f.call(json!({ "copy": "copy", "entity": "hero", "ops": [{ "op": "fill", "region": { "x": 0, "y": 14, "w": 16, "h": 1 }, "index": 2 }] }));
    assert_eq!(r3["written"], true, "{r3}");
    let (r, err) = f.call(json!({ "copy": "copy", "undo": true, "seq": 2 }));
    assert!(err, "{r}");
    assert!(
        r["error"].as_str().unwrap().contains("undo it first"),
        "{r}"
    );
}

#[test]
fn palette_ops_place_colors_with_their_role() {
    let f = Fixture::new();
    let (r, err) = f.call(json!({
        "copy": "copy", "entity": "hero", "palette_ops": [{ "op": "color", "index": 2, "nes": 42 }],
    }));
    assert!(!err, "{r}");
    assert_eq!(r["written"], true, "{r}");
    assert_eq!(r["palettes_changed"], json!([4]), "{r}");
    assert_eq!(r["colors"]["after"], json!([0x0F, 0x01, 42, 0x21]));
    // The buddy draws with palette 4 too.
    assert_eq!(r["shared_with"], json!(["buddy"]), "{r}");
    let (u, err) = f.call(json!({ "copy": "copy", "undo": true }));
    assert!(!err, "{u}");
    let text = fs::read_to_string(f.config.work.join("copy/assets/palettes.toml")).unwrap();
    assert_eq!(
        crate::place::read_palettes(&text).unwrap()[4],
        [0x0F, 0x01, 0x11, 0x21]
    );
    assert!(f.call(json!({ "copy": "copy", "entity": "hero", "ops": edit_ops(), "palette_ops": [{ "op": "hue", "by": 1 }] })).1);
}

#[test]
fn a_sprite_grows_into_free_tiles_and_shrinks_back_on_undo() {
    let f = Fixture::new();
    // Every tile of the made-up sheet is drawn in, so none is free.
    let (r, err) = f.call(json!({ "copy": "copy", "entity": "hero", "ops": [], "resize": [2, 3] }));
    assert!(
        err && r["error"].as_str().unwrap().contains("needs 6 tiles"),
        "{r}"
    );
    // Blank tiles 4 (the ground metatile draws it), 10 and 11.
    let lifted = f.config.assets.clone().unwrap().join("tiles.png");
    let mut px = crate::testgame::sheet(1);
    for t in [4usize, 10, 11] {
        for y in 0..8 {
            for x in 0..8 {
                px[((t / 16) * 8 + y) * 128 + (t % 16) * 8 + x] = 0;
            }
        }
    }
    fs::write(&lifted, gp_assets::chr::write_png(128, 16, &px)).unwrap();
    let (r, err) = f.call(json!({ "copy": "copy", "entity": "hero", "ops": [], "resize": [2, 3] }));
    assert!(!err, "{r}");
    assert_eq!(r["written"], true, "{r}");
    assert_eq!(
        r["resize"]["tiles"],
        json!([16, 17, 18, 19, 10, 11]),
        "tile 4 is drawn by the level: {r}"
    );
    let model = fs::read_to_string(f.config.work.join("copy/model.toml")).unwrap();
    assert!(
        model.contains("tiles = [16, 17, 18, 19, 10, 11]"),
        "{model}"
    );
    assert_eq!(hero_rows(&f).len(), 24);
    assert_eq!(
        hero_rows(&f)[0],
        "0".repeat(16),
        "the art keeps its bottom left"
    );
    let (u, err) = f.call(json!({ "copy": "copy", "undo": true }));
    assert!(!err, "{u}");
    let model = fs::read_to_string(f.config.work.join("copy/model.toml")).unwrap();
    assert!(model.contains("tiles = [16, 17, 18, 19]"), "{model}");
    assert_eq!(hero_rows(&f).len(), 16);
}

#[test]
fn ops_run_on_every_frame_of_a_sprite_set() {
    let f = Fixture::new();
    // The hero gets a walk clip of two frames of their own tiles.
    let model = f.config.work.join("copy/model.toml");
    let text = fs::read_to_string(&model).unwrap().replace(
        "tiles = [16, 17, 18, 19]\nwidth = 2\n",
        "tiles = [16, 17, 18, 19]\nwidth = 2\nclips = [{ when = { moving = true }, every = 8, frames = [{ tiles = [16, 17, 18, 19] }, { tiles = [12, 13, 14, 15] }] }]\n",
    );
    fs::write(&model, text).unwrap();
    let (r, err) = f.call(json!({
        "copy": "copy", "entity": "hero", "frames": ["all"],
        "ops": [{ "op": "fill", "region": { "x": 0, "y": 0, "w": 16, "h": 1 }, "index": 2 }],
    }));
    assert!(!err, "{r}");
    assert_eq!(r["written"], true, "{r}");
    let frames: Vec<&str> = r["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["frame"].as_str().unwrap())
        .collect();
    assert_eq!(frames, ["base", "clip0.1"], "{r}");
    assert_eq!(r["tiles_changed"], json!([12, 13, 16, 17]), "{r}");
    // A tween of the two frames into the clip's second frame.
    let (r, err) = f.call(json!({
        "copy": "copy", "entity": "hero", "frames": ["clip0.1"],
        "ops": [{ "op": "tween", "from": "base", "to": "clip0.1" }], "place": false,
    }));
    assert!(!err, "{r}");
    let (u, err) = f.call(json!({ "copy": "copy", "undo": true }));
    assert!(!err, "{u}");
    assert_eq!(u["undone"]["seq"], 1);
}

/// The hero drawn from a made-up sprite-set (docs/formats/sprite-set.md):
/// two walk frames whose top halves are tile 16 and its mirror, and whose
/// legs swap, as a game's sprite tables give them.
fn hero_from_a_sprite_set(f: &Fixture) {
    let lifted = f.config.assets.clone().unwrap();
    let mut map = fs::read_to_string(lifted.join("assets.toml")).unwrap();
    map.push_str("\n[[asset]]\nname = \"sprites\"\nformat = \"sprite-set\"\n");
    fs::write(lifted.join("assets.toml"), map).unwrap();
    fs::write(
        lifted.join("sprites.toml"),
        r#"schema = 1
asset = "sprites"

[[frame]]
entity = "hero"
state = "walk"
pieces = [{ tile = 16, slot = "tiles", x = 0, y = 0, palette = 4 }, { tile = 16, slot = "tiles", x = 8, y = 0, palette = 4, flip_x = true }, { tile = 18, slot = "tiles", x = 0, y = 8, palette = 4 }, { tile = 19, slot = "tiles", x = 8, y = 8, palette = 4 }]

[[frame]]
entity = "hero"
state = "walk"
pieces = [{ tile = 16, slot = "tiles", x = 0, y = 0, palette = 4 }, { tile = 16, slot = "tiles", x = 8, y = 0, palette = 4, flip_x = true }, { tile = 19, slot = "tiles", x = 0, y = 8, palette = 4, flip_x = true }, { tile = 18, slot = "tiles", x = 8, y = 8, palette = 4, flip_x = true }]
"#,
    )
    .unwrap();
    let model = f.config.work.join("copy/model.toml");
    let text = fs::read_to_string(&model)
        .unwrap()
        .replace(
            "[entities.hero.sprite]\npalette = 4\nslot = \"tiles\"\ntiles = [16, 17, 18, 19]\nwidth = 2\n",
            "[entities.hero.sprite]\npalette = 4\nslot = \"tiles\"\nwidth = 2\nframe = { slot = \"sprites\", row = 0 }\nclips = [{ when = { moving = true }, every = 8, frames = [{ row = 0 }, { row = 1 }] }]\n",
        )
        .replace(
            "[assets.palettes]\n",
            "[assets.sprites]\nkind = \"sprite-set\"\nrole = \"graphics\"\n\n[assets.sprites.from]\nsource = \"tiny\"\nasset = \"sprites\"\n\n[assets.palettes]\n",
        );
    assert!(
        text.contains("row = 1"),
        "the fixture's model changed: {text}"
    );
    fs::write(&model, text).unwrap();
}

#[test]
fn a_clip_of_sprite_set_frames_is_edited_consistently() {
    let f = Fixture::new();
    hero_from_a_sprite_set(&f);
    // A recolor of the whole walk clip: the shared tiles agree.
    let (r, err) = f.call(json!({
        "copy": "copy", "entity": "hero", "frames": ["clip0"],
        "ops": [{ "op": "recolor", "pairs": [[1, 2]] }],
    }));
    assert!(!err, "{r}");
    assert_eq!(r["written"], true, "{r}");
    let ids: Vec<&str> = r["sprite_set"]["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["frame"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["base", "clip0.1"], "{r}");
    let edited: Vec<&str> = r["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["frame"].as_str().unwrap())
        .collect();
    assert_eq!(
        edited,
        ["base", "clip0.1"],
        "clip0's frames, row 0 being the base"
    );
    assert_eq!(r["tiles_changed"], json!([16, 18, 19]), "{r}");
    assert_eq!(r["simulation"]["unchanged"], true);

    // Drawing on one half of the mirrored top can't hold: tile 16 is both
    // halves.
    let (r, err) = f.call(json!({
        "copy": "copy", "entity": "hero", "ops": [{ "op": "pixels", "set": [[0, 0, 3]] }],
    }));
    assert!(err, "{r}");
    assert_eq!(r["stage"], "frames", "{r}");
    // The legs of the base frame are the second frame's legs mirrored, so
    // an edit of the base's legs changes that frame too, and says so.
    let (r, err) = f.call(json!({
        "copy": "copy", "entity": "hero",
        "ops": [{ "op": "fill", "region": { "x": 0, "y": 15, "w": 16, "h": 1 }, "index": 3 }],
    }));
    assert!(!err, "{r}");
    assert_eq!(r["also_changes"], json!(["clip0.1"]), "{r}");
    let (u, err) = f.call(json!({ "copy": "copy", "undo": true }));
    assert!(!err, "{u}");
}

#[test]
fn a_sprite_set_exported_by_the_image_config_comes_back_as_an_upload() {
    let f = Fixture::new();
    let r = tools::call(
        &f.config,
        "assets_export",
        json!({ "copy": "copy", "entity": "hero" }),
    )
    .unwrap();
    assert!(!r.is_error, "{}", r.json);
    let dir = std::path::PathBuf::from(r.json["set"]["dir"].as_str().unwrap());
    assert!(
        dir.join("sprites.json").is_file() && dir.join("palette.json").is_file(),
        "{}",
        r.json
    );
    // Brought back unchanged: nothing to place.
    let (u, err) = f.call(json!({ "copy": "copy", "import": dir }));
    assert!(!err, "{u}");
    assert_eq!(u["pixels_changed"], 0, "{u}");
    // The person draws a pixel of color 2 and saves.
    let side: gp_imagefmt::sheet::Sidecar =
        serde_json::from_slice(&fs::read(dir.join("sprites.json")).unwrap()).unwrap();
    let palette: [[u8; 3]; 16] =
        std::array::from_fn(|i| gp_imagefmt::palette::parse_hex(&side.palette[i]).unwrap());
    let mut got = gp_imagefmt::sheet::read(&|n| fs::read(dir.join(n)).ok(), None).unwrap();
    let e = got.frames[0].px[17];
    got.frames[0].px[17] = (e / 4) * 4 + 2;
    for (name, bytes) in gp_imagefmt::sheet::write(&side.config, "copy", &got.frames, &palette) {
        if name.ends_with(".png") {
            fs::write(dir.join(name), bytes).unwrap();
        }
    }
    let (r, err) = f.call(json!({ "copy": "copy", "import": dir }));
    assert!(!err, "{r}");
    assert_eq!(r["written"], true, "{r}");
    assert_eq!(f.slot()["origin"], "uploaded");
    assert!(
        f.slot()["generator"]
            .as_str()
            .unwrap()
            .starts_with("pixelorama"),
        "{}",
        f.slot()
    );
}

/// The made-up game's "ground" tile kind's picture: its four tiles laid
/// out 2 by 2.
fn ground_rows(f: &Fixture) -> Vec<String> {
    let own = f.config.work.join("copy/assets/tiles.png");
    let file = if own.exists() {
        own
    } else {
        f.config.assets.clone().unwrap().join("tiles.png")
    };
    let sheet = gp_assets::chr::read_png(&fs::read(file).unwrap(), (128, 16)).unwrap();
    let frame = crate::sprite_set::Frame::grid("base", &[1, 2, 3, 4], 2, &[]);
    crate::sprite_set::read(&frame, &sheet, 128, 16)
        .unwrap()
        .rows()
}

#[test]
fn a_level_tile_kinds_look_is_proposed_accepted_and_undone() {
    let f = Fixture::new();
    let original = ground_rows(&f);
    // A tile kind the asset map doesn't have is refused with its names.
    let (r, err) =
        f.call(json!({ "copy": "copy", "tile": "lava", "ops": [{ "op": "flip", "axis": "y" }] }));
    assert!(
        err && r["error"]
            .as_str()
            .unwrap()
            .contains("ground (metatiles[1])"),
        "{r}"
    );
    // Ops that read an entity's sprite don't apply to a tile.
    let (r, err) = f.call(json!({ "copy": "copy", "tile": "ground", "ops": [{ "op": "match_style", "entity": "hero" }] }));
    assert!(err, "{r}");

    let (p, err) = f.call(json!({
        "copy": "copy", "tile": "ground", "place": false, "prompt": "darker ground",
        "ops": [{ "op": "recolor", "pairs": [[1, 2]] }, { "op": "outline", "index": 3 }],
    }));
    assert!(!err, "{p}");
    assert_eq!(p["status"], "proposed", "{p}");
    assert_eq!(p["item"], "copy:1");
    assert_eq!(p["tile"]["tile"], "ground");
    assert_eq!(p["tile"]["tiles"], json!([1, 2, 3, 4]));
    assert_eq!(p["readiness"]["ready"], true, "{p}");
    assert!(p["contexts"]["seen"].as_u64().unwrap() > 0, "{p}");
    // The buddy's sprite draws tiles 1 to 3 of the same sheet.
    assert_eq!(p["also_changes_sprites"], json!(["buddy"]), "{p}");
    assert!(p["pixels_changed"].as_u64().unwrap() > 0);
    assert!(
        p["in_context"]["shots"]
            .as_array()
            .is_some_and(|s| !s.is_empty()),
        "{p}"
    );
    assert!(f.config.work.join("copy/previews/edit-1.png").is_file());
    assert!(
        f.config
            .work
            .join("copy/previews/edit-1-context.png")
            .is_file()
    );
    assert!(
        !f.config.work.join("copy/assets").exists(),
        "a proposal writes no art"
    );

    let (a, err) = f.call(json!({ "copy": "copy", "accept": 1, "author": "person" }));
    assert!(!err, "{a}");
    assert_eq!(a["status"], "accepted", "{a}");
    assert_eq!(a["written"], true, "{a}");
    assert_ne!(ground_rows(&f), original);
    let h = tools::call(&f.config, "assets_history", json!({ "copy": "copy" }))
        .unwrap()
        .json;
    assert_eq!(h["placed"][0]["tile"], "ground", "{h}");

    let (u, err) = f.call(json!({ "copy": "copy", "undo": true }));
    assert!(!err, "{u}");
    assert_eq!(ground_rows(&f), original);
    let (r, err) = f.call(json!({ "copy": "copy", "redo": true }));
    assert!(!err, "{r}");
    assert_ne!(ground_rows(&f), original);

    // Its colors: the background palette the level draws it in.
    let (c, err) = f.call(json!({
        "copy": "copy", "tile": "metatiles[1]", "palette_ops": [{ "op": "color", "index": 1, "nes": 0x27 }],
    }));
    assert!(!err, "{c}");
    assert_eq!(c["palette"], 0, "{c}");
    assert_eq!(c["colors"]["after"], json!([0x0F, 0x27, 0x11, 0x21]), "{c}");
    assert_eq!(c["also_changes"], json!(["sky"]), "{c}");
    assert_eq!(c["written"], true, "{c}");
}
