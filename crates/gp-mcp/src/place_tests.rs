//! `assets_place`, `assets_generate` and `build_publish` on a made-up game:
//! a model with a 20-tile sheet (a 128 by 16 image with room past the last
//! tile), a palettes slot, a second tiles slot on the same asset, one
//! baseline and a stand-in for `gp` whose model check can be made to fail.
//! No game data and no `gp` binary.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::testgame::{MODEL, sheet};
use crate::{Config, catalog, tools};

struct Fixture {
    _dir: tempfile::TempDir,
    config: Config,
    gp: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (config, gp) = crate::testgame::config(dir.path());
        Self {
            _dir: dir,
            config,
            gp,
        }
    }

    fn call(&self, tool: &str, args: Value) -> (Value, bool) {
        let r = tools::call(&self.config, tool, args).expect("a listed tool");
        (r.json, r.is_error)
    }

    /// Makes every later model check fail.
    fn fail_checks(&self) {
        fs::write(self.gp.with_file_name("fail"), "").unwrap();
    }

    fn own(&self) -> PathBuf {
        self.config.work.join("copy/assets")
    }
}

fn png_base64(px: &[u8], w: u32, h: u32) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(gp_assets::chr::write_png(w, h, px))
}

fn tiles_args(px: &[u8], w: u32, h: u32) -> Value {
    json!({
        "copy": "copy", "slot": "tiles", "png_base64": png_base64(px, w, h),
        "license": "CC0-1.0", "origin": "uploaded",
    })
}

fn state(f: &Fixture) -> Value {
    let s = catalog::State::read(&f.own()).unwrap().unwrap();
    serde_json::to_value(s.slots).unwrap()
}

#[test]
fn a_whole_slot_is_placed_and_the_pixels_past_the_last_tile_are_refused() {
    let f = Fixture::new();
    // The sheet is 16 tiles wide and 2 rows: tiles 20 to 31 are unused.
    let mut px = sheet(2);
    let (r, err) = f.call("assets_place", tiles_args(&px, 128, 16));
    assert!(!err, "{r}");
    assert_eq!(r["written"], true);
    assert_eq!(r["simulation"]["unchanged"], true);

    let own = fs::read(f.own().join("tiles.png")).unwrap();
    px[8 * 128 + 8 * 5 + 1] = 3; // row 1, tile column 5 = tile 21
    let before = fs::read(f.own().join("slots.toml")).unwrap();
    let (r, err) = f.call("assets_place", tiles_args(&px, 128, 16));
    assert!(err, "{r}");
    assert_eq!(r["written"], false);
    assert!(
        r["problems"][0]
            .as_str()
            .unwrap()
            .contains("past the last tile"),
        "{r}"
    );
    assert_eq!(fs::read(f.own().join("tiles.png")).unwrap(), own);
    assert_eq!(fs::read(f.own().join("slots.toml")).unwrap(), before);
}

#[test]
fn a_refusal_after_the_write_puts_both_files_back() {
    let f = Fixture::new();
    // Let the copy's assets exist, with their state.
    let px = sheet(2);
    let (r, err) = f.call("assets_place", tiles_args(&px, 128, 16));
    assert!(!err, "{r}");
    let png = fs::read(f.own().join("tiles.png")).unwrap();
    let slots = fs::read(f.own().join("slots.toml")).unwrap();

    f.fail_checks();
    let (r, err) = f.call("assets_place", tiles_args(&sheet(3), 128, 16));
    assert!(err, "{r}");
    assert_eq!(r["written"], false);
    assert_eq!(r["problems"][0], "made to fail", "{r}");
    assert_eq!(fs::read(f.own().join("tiles.png")).unwrap(), png);
    assert_eq!(fs::read(f.own().join("slots.toml")).unwrap(), slots);
    assert!(
        fs::read_dir(f.own()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("tmp")),
        "no temporary file is left"
    );
}

#[test]
fn slots_on_one_asset_share_its_state() {
    let f = Fixture::new();
    let (r, err) = f.call(
        "assets_place",
        json!({
            "copy": "copy", "slot": "tiles", "entity": "hero",
            "png_base64": png_base64(&vec![2u8; 16 * 16], 16, 16),
            "license": "CC0-1.0", "origin": "generated",
        }),
    );
    assert!(!err, "{r}");
    assert_eq!(r["tiles_changed"], json!([16, 17, 18, 19]));
    let s = state(&f);
    assert_eq!(s["tiles"], s["tiles_again"], "{s}");
    assert_eq!(s["tiles"]["status"], "replaced");
    // The catalog finds both slots' files as recorded.
    let (d, _) = f.call("surface_describe", json!({ "copy": "copy" }));
    let slots = d["slots"].as_array().unwrap();
    assert_eq!(slots.len(), 5);
    for slot in slots {
        assert_ne!(slot["status"], "modified", "{slot}");
    }
}

#[test]
fn a_palette_slot_takes_palettes_whole_or_for_one_entity() {
    let f = Fixture::new();
    let (r, err) = f.call(
        "assets_place",
        json!({
            "copy": "copy", "slot": "palettes", "entity": "hero",
            "palettes": [[0x0F, 0x16, 0x27, 0x30]],
            "license": "CC0-1.0", "origin": "uploaded",
        }),
    );
    assert!(!err, "{r}");
    assert_eq!(r["palettes_changed"], json!([4]));
    // The buddy draws with palette 4 too.
    assert_eq!(r["shared_with"], json!(["buddy"]));
    let text = fs::read_to_string(f.own().join("palettes.toml")).unwrap();
    let p = crate::place::read_palettes(&text).unwrap();
    assert_eq!(p[4], [0x0F, 0x16, 0x27, 0x30]);
    assert_eq!(p[0], [0x0F, 0x01, 0x11, 0x21]);

    // A color past 63, a palette of three, and the wrong count.
    for (palettes, want) in [
        (json!([[0x0F, 0x16, 0x27, 64]]), "past the last NES color"),
        (json!([[1, 2, 3]]), "has 3 colors"),
        (json!([[1, 2, 3, 4], [1, 2, 3, 4]]), "give one palette"),
    ] {
        let (r, err) = f.call(
            "assets_place",
            json!({
                "copy": "copy", "slot": "palettes", "entity": "hero",
                "palettes": palettes, "license": "CC0-1.0", "origin": "uploaded",
            }),
        );
        assert!(err, "{r}");
        assert!(r["problems"].to_string().contains(want), "{r}");
    }
    let whole = vec![[0x0F, 0x06, 0x16, 0x26]; 8];
    let (r, err) = f.call(
        "assets_place",
        json!({
            "copy": "copy", "slot": "palettes", "palettes": whole,
            "license": "CC0-1.0", "origin": "uploaded",
        }),
    );
    assert!(!err, "{r}");
    assert_eq!(state(&f)["palettes"]["license"], "CC0-1.0");
}

#[test]
fn generated_assets_are_deterministic_and_pass_the_same_checks() {
    let f = Fixture::new();
    let args = |generator: &str, seed: u64, place: bool| {
        json!({
            "copy": "copy", "slot": "tiles", "entity": "hero",
            "generator": generator, "seed": seed, "place": place,
        })
    };
    let (a, err) = f.call("assets_generate", args("shapes", 3, false));
    assert!(!err, "{a}");
    assert_eq!(a["written"], false);
    assert!(!f.own().exists(), "place false writes nothing");
    let (b, _) = f.call("assets_generate", args("shapes", 3, false));
    assert_eq!(a["candidate"], b["candidate"]);
    let (c, _) = f.call("assets_generate", args("shapes", 4, false));
    assert_ne!(a["candidate"], c["candidate"]);

    let (r, err) = f.call("assets_generate", args("shapes", 3, true));
    assert!(!err, "{r}");
    assert_eq!(r["written"], true);
    assert_eq!(r["simulation"]["unchanged"], true);
    let s = state(&f);
    assert_eq!(s["tiles"]["origin"], "generated");
    assert!(
        s["tiles"]["generator"]
            .as_str()
            .unwrap()
            .contains("shapes seed 3")
    );
    // Only the hero's tiles are new: the rest keeps the original's terms.
    assert_eq!(s["tiles"]["license"], "LicenseRef-own-copy AND CC0-1.0");

    // A whole sheet, palettes, and a hue turned.
    let (r, err) = f.call(
        "assets_generate",
        json!({ "copy": "copy", "slot": "tiles", "generator": "shapes" }),
    );
    assert!(!err, "{r}");
    let (r, err) = f.call(
        "assets_generate",
        json!({ "copy": "copy", "slot": "palettes", "generator": "recolor", "hue_shift": 3 }),
    );
    assert!(!err, "{r}");
    let text = fs::read_to_string(f.own().join("palettes.toml")).unwrap();
    assert_eq!(
        crate::place::read_palettes(&text).unwrap()[0],
        [0x0F, 0x04, 0x14, 0x24]
    );
    // Errors name the choices.
    let (r, err) = f.call(
        "assets_generate",
        json!({ "copy": "copy", "slot": "tiles", "generator": "nope" }),
    );
    assert!(err);
    assert!(r["error"].as_str().unwrap().contains("shapes"), "{r}");
    let (r, err) = f.call(
        "assets_generate",
        json!({ "copy": "copy", "slot": "palettes", "generator": "recolor", "hue_shift": 12 }),
    );
    assert!(err, "{r}");
}

#[test]
fn publishing_is_refused_for_commercial_originals_and_changed_files() {
    let f = Fixture::new();
    let out = f.config.work.join("out");
    let args = json!({ "copy": "copy", "out": out, "dry_run": true });

    // The copy has no template.toml, so it counts as a commercial source's:
    // its lifted slots are the person's own copy.
    let (r, err) = f.call("build_publish", args.clone());
    assert!(err, "{r}");
    assert_eq!(r["stage"], "slots");
    let named: Vec<&str> = r["problems"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["slot"].as_str())
        .collect();
    assert!(
        named.contains(&"tiles") && named.contains(&"palettes"),
        "{r}"
    );
    assert!(!out.exists());

    // Stubs replace the presentation slots; the level and its tileset are
    // simulation, so they stay refused until the model has new ones.
    for slot in ["tiles", "palettes"] {
        let (r, err) = f.call(
            "assets_generate",
            json!({ "copy": "copy", "slot": slot, "generator": "stub" }),
        );
        assert!(!err, "{r}");
    }
    let (r, err) = f.call("build_publish", args.clone());
    assert!(err, "{r}");
    let named: Vec<&str> = r["problems"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["slot"].as_str())
        .collect();
    assert_eq!(named, ["level", "metatiles"], "{r}");
    assert_eq!(r["manifest"]["assets"]["slots"]["tiles"]["status"], "stub");

    // A copy of an open-tier template may keep its originals: they're the
    // template's own, and the dry run passes with a manifest.
    let o = Fixture::new();
    fs::write(
        o.config.work.join("copy/template.toml"),
        "[template]\nformat = \"0.1.0\"\nid = \"tiny\"\ntitle = \"Tiny\"\ntier = \"open\"\n",
    )
    .unwrap();
    let (r, err) = o.call(
        "assets_generate",
        json!({ "copy": "copy", "slot": "palettes", "generator": "stub" }),
    );
    assert!(!err, "{r}");
    let (r, err) = o.call(
        "build_publish",
        json!({ "copy": "copy", "out": o.config.work.join("out"), "dry_run": true }),
    );
    assert!(!err, "{r}");
    assert_eq!(r["written"], false);
    assert_eq!(r["manifest"]["model"]["name"], "tiny");
    assert_eq!(r["manifest"]["template"]["tier"], "open");
    assert_eq!(
        r["manifest"]["licenses"]["assets"],
        json!(["CC0-1.0", "LicenseRef-template-source"])
    );
    assert!(!o.config.work.join("out").exists());

    // A file edited by hand no longer matches its recorded digest.
    let path = f.own().join("tiles.png");
    fs::write(
        &path,
        gp_assets::chr::write_png(128, 16, &vec![3u8; 128 * 16]),
    )
    .unwrap();
    let (r, err) = f.call("build_publish", args);
    assert!(err, "{r}");
    assert!(r["problems"].to_string().contains("changed since"), "{r}");
    let (d, _) = f.call("surface_describe", json!({ "copy": "copy" }));
    let tiles = d["slots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "tiles")
        .unwrap();
    assert_eq!(tiles["status"], "modified", "{tiles}");

    // A failing model check is the first refusal.
    f.fail_checks();
    let (r, err) = f.call(
        "build_publish",
        json!({ "copy": "copy", "out": f.config.work.join("out"), "dry_run": true }),
    );
    assert!(err, "{r}");
    assert_eq!(r["stage"], "model check");
}

#[test]
fn calls_on_one_copy_take_turns() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let inside = AtomicUsize::new(0);
    let worst = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                for _ in 0..20 {
                    let _lock = crate::CopyLock::take("same");
                    let n = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    worst.fetch_max(n, Ordering::SeqCst);
                    std::thread::yield_now();
                    inside.fetch_sub(1, Ordering::SeqCst);
                }
            });
        }
    });
    assert_eq!(worst.load(Ordering::SeqCst), 1);
    // Another copy doesn't wait.
    let _a = crate::CopyLock::take("one");
    let _b = crate::CopyLock::take("two");
}

#[test]
fn the_registry_is_searched_by_words_platform_genre_and_roles() {
    let f = Fixture::new();
    let t = f.config.repo.join("templates/tiny");
    fs::create_dir_all(&t).unwrap();
    fs::write(t.join("model.toml"), MODEL).unwrap();
    fs::write(
        t.join("template.toml"),
        "[template]\nformat = \"0.1.0\"\nid = \"tiny\"\ntitle = \"Tiny Hop\"\ntier = \"open\"\nmodel = \"model.toml\"\n\n[source]\nid = \"tiny\"\nplatform = \"nes\"\nhow = \"build\"\nbuild = \"just tiny\"\naccepts = [\"00000000000000000000000000000000000000000000000000000000000000aa\"]\n\n[slots]\ntiles = \"graphics\"\n",
    )
    .unwrap();
    fs::create_dir_all(f.config.repo.join("sources")).unwrap();
    fs::write(
        f.config.repo.join("sources/nes.toml"),
        "[[source]]\nid = \"tiny\"\ntitle = \"Tiny Hop\"\nplatform = \"nes\"\nsha256 = \"00000000000000000000000000000000000000000000000000000000000000aa\"\nstatus = \"verified\"\n\n[[source]]\nid = \"other-game\"\ntitle = \"Other Game\"\nplatform = \"nes\"\nsha256 = \"00000000000000000000000000000000000000000000000000000000000000bb\"\n",
    )
    .unwrap();
    let search = |args: Value| f.call("registry_search", args).0;

    let r = search(
        json!({ "query": "tiny hop", "platform": "nes", "genre": "platformer", "roles": ["player", "npc"] }),
    );
    assert_eq!(r["templates_found"], 1, "{r}");
    let t = &r["templates"][0];
    assert_eq!(t["template"], "tiny");
    assert_eq!(t["roles"], json!({ "npc": 1, "player": 1 }));
    // The lifted assets are at hand (the server was given them); the ROM
    // isn't, and isn't needed once they are.
    assert_eq!(t["ready"], true, "{t}");
    assert_eq!(t["needs"]["rom"]["present"], false);
    assert_eq!(t["needs"]["extracted"]["present"], true);
    // Registered sources have no model: a genre or role filter leaves
    // them out.
    assert_eq!(r["sources_found"], 0, "{r}");
    let r = search(json!({ "query": "tiny", "platform": "nes" }));
    assert_eq!(r["sources_found"], 1, "{r}");
    assert_eq!(r["sources"][0]["templates"], json!(["tiny"]));
    assert_eq!(r["sources"][0]["rom_present"], false);

    for none in [
        json!({ "query": "zelda" }),
        json!({ "platform": "snes" }),
        json!({ "genre": "adventure" }),
        json!({ "roles": ["boss"] }),
        json!({ "systems": ["nope"] }),
    ] {
        let r = search(none.clone());
        assert_eq!(r["templates_found"], 0, "{none}: {r}");
    }
    // Without a model filter the sources are searched too.
    let r = search(json!({ "query": "other" }));
    assert_eq!(r["templates_found"], 0);
    assert_eq!(r["sources"][0]["id"], "other-game");
}

#[test]
fn the_critic_reviews_candidates_and_can_hold_a_placement() {
    let f = Fixture::new();
    let base = json!({ "copy": "copy", "slot": "tiles", "entity": "hero", "generator": "shapes", "seed": 3 });
    let with = |extra: Value| {
        let mut a = base.clone();
        a.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        f.call("assets_generate", a)
    };
    // Advise is the default: the review comes back and the candidate is placed.
    let (r, err) = with(json!({ "place": false }));
    assert!(!err, "{r}");
    assert_eq!(r["critic"]["critic"], "metrics");
    let names: Vec<&str> = r["critic"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"shape.ink") && names.contains(&"contrast.backdrop"),
        "{names:?}"
    );
    let again = with(json!({ "place": false })).0;
    assert_eq!(r["critic"], again["critic"], "a review is deterministic");
    // Off means no review.
    let (r, _) = with(json!({ "place": false, "critic": "off" }));
    assert!(r["critic"].is_null(), "{r}");
    // Enforce with a score nothing reaches writes nothing.
    let (r, err) = with(json!({ "critic": "enforce", "min_score": 1.5 }));
    assert!(!err, "{r}");
    assert_eq!(r["written"], false);
    assert_eq!(r["stage"], "critic");
    assert!(!f.own().exists(), "{r}");
    // Enforce at the default score places what passes.
    let (r, err) = with(json!({ "critic": "enforce", "min_score": 0.0 }));
    assert!(!err && r["written"] == true, "{r}");
    let (r, err) = with(json!({ "critic": "sometimes" }));
    assert!(err, "{r}");
}

#[test]
fn progressive_generation_fills_palettes_then_sprites_by_role() {
    let f = Fixture::new();
    let (r, err) = f.call(
        "assets_generate",
        json!({ "copy": "copy", "progressive": true, "plan_only": true }),
    );
    assert!(!err, "{r}");
    let order: Vec<(String, String)> = r["plan"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["slot"].as_str().unwrap().to_owned(),
                s["entity"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    let want = [
        ("palettes", ""),
        ("tiles", ""),
        ("tiles", "hero"),
        ("tiles", "buddy"),
    ];
    assert_eq!(
        order,
        want.map(|(a, b)| (a.to_owned(), b.to_owned())),
        "palettes, the base sheet (tiles_again shares its asset), the player, then the npc"
    );
    assert!(!f.own().exists(), "a plan writes nothing");

    let (r, err) = f.call(
        "assets_generate",
        json!({ "copy": "copy", "progressive": true, "tries": 3, "min_score": 0.5 }),
    );
    assert!(!err, "{r}");
    assert_eq!(r["steps"], 4);
    let done = r["done"].as_array().unwrap();
    assert!(done.iter().all(|d| d["written"] == true), "{r}");
    assert!(
        done.iter().all(|d| d["simulation_unchanged"] == true),
        "{r}"
    );
    let s = state(&f);
    assert_eq!(s["tiles"]["origin"], "generated");
    assert_eq!(s["palettes"]["origin"], "stub");
    // Run again: the base sheet is no longer original, so it isn't redone.
    let (r, _) = f.call(
        "assets_generate",
        json!({ "copy": "copy", "progressive": true, "plan_only": true }),
    );
    assert_eq!(r["plan"].as_array().unwrap().len(), 3, "{r}");
}

#[test]
fn the_manifest_says_what_the_copy_changed_from_its_template() {
    let f = Fixture::new();
    let tdir = f.config.repo.join("templates/tiny");
    fs::create_dir_all(&tdir).unwrap();
    fs::write(tdir.join("model.toml"), MODEL).unwrap();
    fs::write(
        f.config.work.join("copy/template.toml"),
        "[template]\nformat = \"0.1.0\"\nid = \"tiny\"\ntitle = \"Tiny\"\ntier = \"open\"\n[parent]\ncontent_sha256 = \"abc\"\n",
    )
    .unwrap();
    let out = f.config.work.join("out");
    let args = json!({ "copy": "copy", "out": out, "dry_run": true });
    let (r, err) = f.call("build_publish", args.clone());
    assert!(!err, "{r}");
    let m = &r["manifest"];
    assert_eq!(m["format"], "0.2.0");
    assert_eq!(m["parent"]["template"], "tiny");
    assert_eq!(m["parent"]["content_sha256"], "abc");
    assert_eq!(m["changes"]["model"].as_array().unwrap().len(), 0, "{m}");

    let (r, err) = f.call(
        "model_edit",
        json!({ "copy": "copy", "edits": [{ "path": "entities.hero.jump.gravity", "value": "0.1875" }] }),
    );
    assert!(!err, "{r}");
    let (r, err) = f.call("build_publish", args);
    assert!(!err, "{r}");
    let changes = r["manifest"]["changes"]["model"].as_array().unwrap();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0]["path"], "entities.hero.jump.gravity");
    assert!(r["manifest"]["wasm"].is_null(), "a dry run builds nothing");
}

#[test]
fn describe_returns_the_templates_recipes_and_scenario_names() {
    let f = Fixture::new();
    let (r, err) = f.call("surface_describe", json!({ "copy": "copy" }));
    assert!(!err, "{r}");
    assert!(r["recipes"].is_null());
    assert_eq!(r["scenarios"].as_array().unwrap().len(), 0);
    fs::write(f.config.work.join("copy/AGENTS.md"), "# Changing Tiny\n").unwrap();
    fs::write(
        f.config.work.join("copy/scenarios.toml"),
        "format = \"0.1.0\"\n[scenarios.walks]\ndescription = \"Walks\"\nframes = 1\n",
    )
    .unwrap();
    let (r, _) = f.call("surface_describe", json!({ "copy": "copy" }));
    assert_eq!(r["recipes"], "# Changing Tiny\n");
    assert_eq!(r["scenarios"][0]["name"], "walks");
    assert_eq!(r["scenarios"][0]["description"], "Walks");
}

#[test]
fn validate_can_run_the_copys_scenarios() {
    let f = Fixture::new();
    fs::write(
        f.config.work.join("copy/scenarios.toml"),
        r#"format = "0.1.0"

[scenarios.stands]
frames = 5

[[scenarios.stands.expect]]
path = "entities.hero[0].alive"
op = "eq"
value = "true"

[scenarios.never]
frames = 5

[[scenarios.never.expect]]
path = "entities.hero[0].x"
op = "gt"
value = "1000"
"#,
    )
    .unwrap();
    let (r, err) = f.call("validate", json!({ "copy": "copy" }));
    assert!(!err, "{r}");
    assert!(r["scenarios"].is_null(), "off unless asked");
    let (r, err) = f.call("validate", json!({ "copy": "copy", "scenarios": true }));
    assert!(!err, "{r}");
    assert_eq!(r["scenarios"]["passed"], 1, "{r}");
    assert_eq!(r["scenarios"]["failed"], 1);
    let failing: Vec<&Value> = r["scenarios"]["results"][0]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert_eq!(failing.len(), 1, "only failing expectations are returned");
    let (r, _) = f.call(
        "validate",
        json!({ "copy": "copy", "scenarios": true, "scenario_names": ["stands"] }),
    );
    assert_eq!(r["scenarios"]["ok"], true, "{r}");
}

/// An editor that saves RGBA: the four preview grays read as indices
/// (gp-imagefmt), and any other color is refused.
#[test]
fn an_rgba_image_of_the_preview_grays_is_placed() {
    use base64::Engine as _;
    let f = Fixture::new();
    let px = sheet(2);
    let encode = |rgba: &[u8]| {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 128, 16);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(rgba).unwrap();
        }
        base64::engine::general_purpose::STANDARD.encode(out)
    };
    let mut rgba: Vec<u8> = px
        .iter()
        .flat_map(|&i| [i * 0x55, i * 0x55, i * 0x55, 255])
        .collect();
    let args = |b64: String| json!({ "copy": "copy", "slot": "tiles", "png_base64": b64, "license": "CC0-1.0", "origin": "uploaded" });
    let (r, err) = f.call("assets_place", args(encode(&rgba)));
    assert!(!err, "{r}");
    assert_eq!(r["written"], true);
    rgba[0] = 7;
    let (r, err) = f.call("assets_place", args(encode(&rgba)));
    assert!(err, "{r}");
    assert!(r.to_string().contains("not one of the four grays"), "{r}");
}
