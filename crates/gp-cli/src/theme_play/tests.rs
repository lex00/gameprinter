//! `gp theme look play` on a made-up game, registry and theme folder.

use super::*;

/// A made-up page-check.toml: two ROM versions' routes, one that names a
/// theme-shots.toml run (passed over here) and a death list the moments
/// leave out.
const PAGE_CHECK: &str = r#"
[page]
characters = ["someone"]

[[route]]
rom = "made-up-a"
plays = ["main", "m2"]
ending = { frame = 70, kinds = ["heart"] }
again = { frame = 80, player = [1, 2] }
frames = 90
holds = ["start@1-2", "right@5-8"]

[[route.stage]]
name = "1st"
level = "main"
start = 10
player = [1, 2]
shot = 20
kinds = ["a"]
clear = 30
top = 4

[[route.stage]]
name = "2nd"
level = "m2"
start = 40
shot = 50
clear = 60
top = 4

[[route.death]]
name = "a fall: off the edge"
holds = ["left@3-4"]
hit = 9

[[route]]
rom = "made-up-b"
frames = 50
holds = ["a@1-1"]
ending = { frame = 40 }

[[route]]
rom = "made-up-c"
run = "c-route"
"#;

const PICTURES: &str = "# made up\n\nmoment a-walk\n  section pictures-rom-aaaa1111\n  frame   15\n  shows   walking\n\nmoment a-end-night\n  section pictures-rom-aaaa1111\n  frame   70\n  look    night\n\nmoment a-late-night\n  section pictures-rom-aaaa1111\n  frame   75\n  look    night\n\nmoment a-jump\n  section pictures-rom-aaaa1111\n  frame   25\n  press   a@22-23\n  shows   a jump\n\nmoment b-only\n  section pictures-rom-bbbb2222\n  frame   12\n";

fn roms() -> Vec<(String, String)> {
    vec![
        ("made-up-a".into(), "aaaa1111ffff".into()),
        ("made-up-b".into(), "bbbb2222ffff".into()),
        ("made-up-c".into(), "cccc3333ffff".into()),
    ]
}

#[test]
fn the_page_check_gives_each_rom_its_route_and_moments() {
    let m = page_check_moments(PAGE_CHECK, PICTURES, &roms()).unwrap();
    m.check().unwrap();
    let names: Vec<(&str, &str, u32)> = m
        .shot
        .iter()
        .map(|s| (s.name.as_str(), s.run.as_str(), s.frame))
        .collect();
    assert_eq!(
        names,
        vec![
            ("1st-start", "made-up-a-route", 10),
            ("1st", "made-up-a-route", 20),
            ("1st-clear", "made-up-a-route", 30),
            ("2nd-start", "made-up-a-route", 40),
            ("2nd", "made-up-a-route", 50),
            ("2nd-clear", "made-up-a-route", 60),
            ("ending", "made-up-a-route", 70),
            ("again", "made-up-a-route", 80),
            ("a-walk", "made-up-a-route", 15),
            // The ending at 70 is there already; a look's name is left out.
            ("a-late", "made-up-a-route", 75),
            ("a-jump", "made-up-a-a-jump", 25),
            ("ending", "made-up-b-route", 40),
            ("b-only", "made-up-b-route", 12),
        ]
    );
    // A picture with presses plays its own run: the route and the presses.
    let jump = m.run.iter().find(|r| r.name == "made-up-a-a-jump").unwrap();
    assert_eq!(jump.holds, vec!["start@1-2", "right@5-8", "a@22-23"]);
    assert_eq!(jump.rom, "made-up-a");
    // A ROM the page check has no route of its own for has no moments.
    assert!(m.run.iter().all(|r| r.rom != "made-up-c"));
}

#[test]
fn holds_become_a_frames_file() {
    let f = frames_of(
        &["start@1-2".into(), "right+a@2-3".into(), "up@4-".into()],
        6,
    )
    .unwrap();
    assert_eq!(f, "gp-frames 1\n00\n08\n89\n81\n10\n10\n");
    assert!(frames_of(&["jump@1-2".into()], 4).is_err());
}

#[test]
fn a_moments_file_is_read_and_a_shot_past_its_run_refused() {
    let text = r#"
[[run]]
name = "r"
rom = "made-up-a"
frames = 100
holds = ["start@1-2"]

[[shot]]
name = "walk"
run = "r"
frame = 50
shows = "walking"
"#;
    let m: Moments = toml::from_str(text).unwrap();
    m.check().unwrap();
    let mut bad = m.clone();
    bad.shot[0].frame = 100;
    assert!(bad.check().is_err());
    let mut bad = m;
    bad.shot[0].run = "nope".into();
    assert!(bad.check().is_err());
}

#[test]
fn the_kept_ports_are_never_taken() {
    for p in TAKEN_PORTS {
        assert!(refuse_port(p).is_err());
        assert!(bind(p).is_err());
    }
    let l = bind(0).unwrap();
    assert!(!TAKEN_PORTS.contains(&l.local_addr().unwrap().port()));
}

#[test]
fn a_pair_is_both_pictures_side_by_side() {
    let mut a = Rgba::new(3, 2);
    a.set(0, 0, [255, 0, 0, 255]);
    let mut b = Rgba::new(2, 3);
    b.set(1, 2, [0, 255, 0, 255]);
    let p = pair(&a, &b);
    assert_eq!((p.width, p.height), (3 + 16 + 2, 3));
    assert_eq!(p.get(0, 0), [255, 0, 0, 255]);
    assert_eq!(p.get(3 + 16 + 1, 2), [0, 255, 0, 255]);
    assert_eq!(p.get(5, 0), [24, 24, 24, 255]);
    // A large-art look beside the console's pictures: the small one is
    // made a whole number of times bigger, each pixel a square.
    let mut small = Rgba::new(2, 2);
    small.set(1, 1, [0, 0, 255, 255]);
    let big = Rgba::new(10, 7);
    let p = pair(&big, &small);
    assert_eq!((p.width, p.height), (10 + 16 + 6, 7));
    assert_eq!(p.get(10 + 16 + 3, 3), [0, 0, 255, 255]);
    assert_eq!(p.get(10 + 16 + 5, 5), [0, 0, 255, 255]);
    assert_eq!(p.get(10 + 16 + 2, 2), [0, 0, 0, 0]);
}

/// A made-up theme folder: package.toml, theme/ and git's files.
fn theme_folder(dir: &Path, name: &str, version: &str) -> PathBuf {
    let root = dir.join("my-theme");
    fs::create_dir_all(root.join("theme/art")).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(
        root.join("package.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nkind = \"content\"\ntitle = \"My Theme\"\n"),
    )
    .unwrap();
    fs::write(root.join("theme/look.txt"), "gp-look 1\n").unwrap();
    fs::write(root.join("theme/art/hero.png"), b"made up").unwrap();
    fs::write(root.join(".git/HEAD"), "ref").unwrap();
    fs::write(root.join(".gitattributes"), "*.png lfs").unwrap();
    root
}

/// A made-up source registry: a game, two versions of another theme and a
/// published version of the folder's own package.
fn source(dir: &Path) -> Folder {
    let t = Folder(dir.to_owned());
    let now = crate::registry::utc(0).unwrap();
    let ns = |n: &str| (n.to_owned(), Vec::new());
    init(
        &t,
        "made-up-registry",
        &Signer::none(),
        &[],
        &[ns("maker"), ns("friend"), ns("me")],
        &now,
    )
    .unwrap();
    let put = |name: &str, version: &str, kind: &str, file: &str| {
        let mut files = BTreeMap::new();
        files.insert(file.to_owned(), format!("{name}@{version}").into_bytes());
        let for_games = if kind == "content" {
            vec!["maker/game".to_owned()]
        } else {
            vec![]
        };
        publish(
            &t,
            &Publish {
                name,
                version,
                kind,
                title: name,
                for_games: &for_games,
                published: &now,
            },
            &files,
            &Signer::none(),
        )
        .unwrap();
    };
    put("maker/game", "1.0.0", "game", "play/module.wasm");
    put("friend/night", "0.9.0", "content", "theme/look.txt");
    put("friend/night", "0.10.0", "content", "theme/look.txt");
    put("me/my-theme", "0.1.0", "content", "theme/look.txt");
    let all: Vec<(String, String)> = [
        ("maker/game", "1.0.0"),
        ("friend/night", "0.9.0"),
        ("friend/night", "0.10.0"),
        ("me/my-theme", "0.1.0"),
    ]
    .iter()
    .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
    .collect();
    reindex(
        &t,
        &all,
        &Signer::none(),
        &now,
        &crate::registry::utc(7).unwrap(),
    )
    .unwrap();
    t
}

fn listed(dir: &Path) -> Vec<(String, String)> {
    let f = Folder(dir.to_owned());
    let r = Reader::unsigned(&f).unwrap();
    let now = crate::registry::utc(0).unwrap();
    r.by_game("maker/game", &now)
        .unwrap()
        .line
        .into_iter()
        .map(|l| (l.package, l.version))
        .collect()
}

#[test]
fn the_scratch_registry_holds_the_game_its_newest_themes_and_the_folder() {
    let d = tempfile::tempdir().unwrap();
    let src = source(&d.path().join("src"));
    let root = theme_folder(d.path(), "me/my-theme", "0.2.0");
    // The theme/ folder names the same theme as its parent.
    let pkg = theme_pkg(&root.join("theme")).unwrap();
    assert_eq!(
        (pkg.name.as_str(), pkg.version.as_str()),
        ("me/my-theme", "0.2.0")
    );
    let out = d.path().join("scratch");
    let b = build_registry(&src, &out, "maker/game", &Side::Folder(pkg.clone()), None).unwrap();
    assert_eq!(b.game_version, "1.0.0");
    assert_eq!(b.themes, vec!["me/my-theme@0.2.0", "friend/night@0.10.0"]);
    // Offered for the game: the folder's version only (not its published
    // one) and the other theme's newest, by number (0.10.0 after 0.9.0).
    let mut l = listed(&out);
    l.sort();
    assert_eq!(
        l,
        vec![
            ("friend/night".to_owned(), "0.10.0".to_owned()),
            ("me/my-theme".to_owned(), "0.2.0".to_owned()),
        ]
    );
    // The folder's package as the page installs it: no git files.
    let f = Folder(out.clone());
    let r = Reader::unsigned(&f).unwrap();
    let v = r.version("me/my-theme", "0.2.0").unwrap();
    let (_, files) = r.fetch("me/my-theme", "0.2.0", &v.id).unwrap();
    let names: Vec<&str> = files.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        vec!["package.toml", "theme/art/hero.png", "theme/look.txt"]
    );
    // The game is there by its published id, every file checked.
    let g = r.version("maker/game", "1.0.0").unwrap();
    assert!(r.fetch("maker/game", "1.0.0", &g.id).is_ok());
    // Nothing was written to the source.
    assert!(
        Reader::unsigned(&src)
            .unwrap()
            .version("me/my-theme", "0.2.0")
            .is_err()
    );

    // A published version against: the same package's other version.
    let out2 = d.path().join("scratch2");
    let side = against_side("me/my-theme@0.1.0").unwrap();
    build_registry(&src, &out2, "maker/game", &side, None).unwrap();
    let mut l = listed(&out2);
    l.sort();
    assert_eq!(l[1], ("me/my-theme".to_owned(), "0.1.0".to_owned()));
    assert!(against_side("not-a-name").is_err());
}

#[test]
fn the_page_is_served_with_the_theme_chosen_and_the_registry_beside_it() {
    use std::io::{Read as _, Write as _};
    let d = tempfile::tempdir().unwrap();
    let page = d.path().join("page");
    fs::create_dir_all(&page).unwrap();
    fs::write(page.join("index.html"), "<p>made up</p>").unwrap();
    fs::write(
        page.join("page.json"),
        r#"{"game":"maker/game","source":{"name":"made-up-registry","url":"https://example.invalid/reg"},"content":"maker/default"}"#,
    )
    .unwrap();
    fs::write(page.join("_headers"), "/*\n  X-Made-Up: yes\n").unwrap();
    let reg = d.path().join("reg");
    fs::create_dir_all(&reg).unwrap();
    fs::write(reg.join("registry.toml"), "name = \"made-up\"\n").unwrap();
    let side = Side::Published {
        name: "friend/night".into(),
        version: "0.9.0".into(),
    };
    let mut runs = BTreeMap::new();
    runs.insert("r".to_owned(), "gp-frames 1\n00\n".to_owned());
    let ov = d.path().join("overlay");
    overlay(&ov, &page, &side, &runs).unwrap();
    let l = bind(0).unwrap();
    let port = l.local_addr().unwrap().port();
    serve_on(
        l,
        &page,
        &Served {
            label: side.words(),
            registry: reg,
            overlay: ov,
            port,
            link: String::new(),
        },
    );
    let get = |path: &str| {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    };
    let pj = get("/page.json");
    assert!(pj.contains("X-Made-Up: yes"), "{pj}");
    let body: Value = serde_json::from_str(pj.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["source"]["url"], ".");
    assert_eq!(body["content"], "friend/night");
    assert_eq!(body["game"], "maker/game");
    assert!(get("/p?g=maker/game").contains("made up"));
    assert!(get("/registry.toml").contains("made-up"));
    assert!(get("/game/runs/r.frames").contains("gp-frames 1"));
    assert!(get("/../etc/passwd").starts_with("HTTP/1.1 404"));
}

#[test]
fn a_page_that_passed_the_release_check_is_found_for_its_game() {
    let d = tempfile::tempdir().unwrap();
    let rel = d.path().join("work/page-release");
    let passed = rel.join("passed");
    fs::create_dir_all(&passed).unwrap();
    for (name, game, when) in [
        ("old", "maker/game", "2026-11-01T00:00:00"),
        ("new", "maker/game", "2026-11-02T00:00:00"),
        ("other", "maker/other", "2026-11-03T00:00:00"),
    ] {
        let b = rel.join(name);
        fs::create_dir_all(&b).unwrap();
        fs::write(b.join("index.html"), "").unwrap();
        fs::write(b.join("page.json"), format!(r#"{{"game":"{game}"}}"#)).unwrap();
        fs::write(
            passed.join(name),
            format!("bundle {}\nid {name}\nwhen {when}\n", b.display()),
        )
        .unwrap();
    }
    assert_eq!(
        passed_bundle(d.path(), "maker/game").unwrap(),
        rel.join("new")
    );
    assert!(passed_bundle(d.path(), "maker/none").is_none());
}

/// A made-up game folder: template.toml with the source's `how` and
/// platform, and a package.toml with its ROM entries' platforms.
fn game_folder(dir: &Path, how: &str, platform: &str, entry: Option<&str>) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("template.toml"),
        format!("[template]\nid = \"made-up\"\ntitle = \"Made Up\"\n\n[source]\nid = \"made-up\"\nplatform = \"{platform}\"\nhow = \"{how}\"\n"),
    )
    .unwrap();
    if let Some(p) = entry {
        fs::write(
            dir.join("package.toml"),
            format!("[package]\nname = \"maker/made-up\"\nversion = \"0.3.0\"\nkind = \"game\"\ntitle = \"Made Up!\"\n\n[[rom.entry]]\nid = \"made-up\"\nsha256 = \"00\"\nplatform = \"{p}\"\n"),
        )
        .unwrap();
    }
    dir.to_owned()
}

#[test]
fn a_game_plays_from_a_rom_from_its_own_files_or_is_refused_in_words() {
    let d = tempfile::tempdir().unwrap();
    let rom = game_folder(&d.path().join("rom"), "rom", "nes", Some("nes"));
    let f = game_facts(&rom).unwrap();
    assert_eq!(
        (f.road, f.id.as_str(), f.title.as_str()),
        (Road::Rom, "made-up", "Made Up")
    );
    assert!(!plays_without_rom(&rom));
    for how in ["build", "authored"] {
        let own = game_folder(&d.path().join(how), how, "nes", None);
        assert_eq!(game_facts(&own).unwrap().road, Road::Own, "{how}");
        assert!(plays_without_rom(&own));
    }
    // An arcade game, by its template or by a ROM entry: played from the
    // player's romset on the any-game page, as a ROM game is.
    for (name, platform, entry) in [("a", "arcade", None), ("b", "nes", Some("arcade"))] {
        let arcade = game_folder(&d.path().join(name), "rom", platform, entry);
        assert_eq!(game_facts(&arcade).unwrap().road, Road::Rom, "{name}");
        assert!(!plays_without_rom(&arcade));
    }
    let odd = game_folder(&d.path().join("odd"), "guess", "nes", None);
    let e = game_facts(&odd).unwrap_err().to_string();
    assert!(
        e.starts_with("Made Up doesn't say where its pictures come from"),
        "{e}"
    );
}

/// An arcade game with no moments of its own: a coin and the start in its
/// own button columns, the attract mode and the first level.
#[test]
fn an_arcade_game_with_no_moments_drops_a_coin_and_starts() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("inputs")).unwrap();
    fs::write(
        d.path().join("inputs/made-up.001.start.presses"),
        "gp-presses 0\nframes 10\narcade madeup SCRLDUT\ncoin@1-2\n",
    )
    .unwrap();
    assert_eq!(arcade_columns(d.path()).as_deref(), Some("SCRLDUT"));
    let only_c = vec![roms()[2].clone()];
    let (m, from) = moments_for(None, d.path(), &only_c).unwrap();
    // Coin is the second column here (b), the start the first (a).
    assert_eq!(m.run[0].holds, vec!["b@124-134", "a@195-201"]);
    assert_eq!(m.shot.len(), 3);
    assert!(from.contains("a coin at frame 124"), "{from}");
    let f = frames_of(&m.run[0].holds, m.run[0].frames).unwrap();
    let pad: Vec<&str> = f.lines().skip(1).collect();
    assert_eq!((pad[124], pad[195], pad[300]), ("02", "01", "00"));
    // An NES game has no columns.
    assert_eq!(arcade_columns(&d.path().join("none")), None);
}

#[test]
fn a_game_with_no_moments_of_its_own_gets_the_title_and_the_first_stage() {
    let m = fallback_moments(&roms()[..2]);
    m.check().unwrap();
    assert_eq!(m.run.len(), 2);
    assert_eq!(m.run[0].holds, vec!["start@150-154"]);
    let names: Vec<(&str, &str, u32)> = m
        .shot
        .iter()
        .map(|s| (s.name.as_str(), s.run.as_str(), s.frame))
        .collect();
    assert_eq!(
        names[..3],
        [
            ("title", "made-up-a-start", 120),
            ("stage-1", "made-up-a-start", 390),
            ("stage-1-later", "made-up-a-start", 690),
        ]
    );
    // A game folder with no theme-shots.toml and ROMs the page check has
    // no route for falls back to them, and says so.
    let d = tempfile::tempdir().unwrap();
    let only_c = vec![roms()[2].clone()];
    let (m, from) = moments_for(None, d.path(), &only_c).unwrap();
    assert_eq!(m.run[0].rom, "made-up-c");
    assert!(from.contains("no theme-shots.toml of its own"), "{from}");
    // Its own file wins.
    fs::write(
        d.path().join(MOMENTS_FILE),
        "[[run]]\nname = \"r\"\nrom = \"made-up-c\"\nframes = 10\nholds = []\n\n[[shot]]\nname = \"s\"\nrun = \"r\"\nframe = 5\n",
    )
    .unwrap();
    let (m, from) = moments_for(None, d.path(), &only_c).unwrap();
    assert_eq!(m.shot[0].name, "s");
    assert!(from.ends_with(MOMENTS_FILE), "{from}");
}

#[test]
fn a_game_the_registry_hasnt_got_is_published_into_the_scratch_registry_alone() {
    let d = tempfile::tempdir().unwrap();
    let src = source(&d.path().join("src"));
    let game = game_folder(&d.path().join("made-up"), "rom", "nes", Some("nes"));
    fs::create_dir_all(game.join("target/debug")).unwrap();
    fs::write(game.join("target/debug/big"), b"build output").unwrap();
    fs::write(game.join("model.toml"), b"made up").unwrap();
    let play = d.path().join("page.play");
    fs::create_dir_all(play.join("lift")).unwrap();
    fs::write(play.join("page.json"), b"{}").unwrap();
    fs::write(play.join("lift/game.json"), b"{}").unwrap();
    let local = LocalGame {
        dir: game,
        play,
        version: "0.3.0".into(),
        title: "Made Up!".into(),
    };
    assert!(!source_has(&src, "maker/made-up").unwrap());
    assert!(source_has(&src, "maker/game").unwrap());
    let root = theme_folder(d.path(), "me/my-theme", "0.2.0");
    let side = Side::Folder(theme_pkg(&root).unwrap());
    let out = d.path().join("scratch");
    let b = build_registry(&src, &out, "maker/made-up", &side, Some(&local)).unwrap();
    assert_eq!(b.game_version, "0.3.0");
    let f = Folder(out.clone());
    let r = Reader::unsigned(&f).unwrap();
    let v = r.version("maker/made-up", "0.3.0").unwrap();
    let (_, files) = r.fetch("maker/made-up", "0.3.0", &v.id).unwrap();
    let names: Vec<&str> = files.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        vec![
            "model.toml",
            "package.toml",
            "play/lift/game.json",
            "play/page.json",
            "template.toml"
        ]
    );
    // The folder's theme is offered for it; the source's game isn't copied.
    assert!(r.version("maker/game", "1.0.0").is_err());
    assert!(r.version("me/my-theme", "0.2.0").is_ok());
    // A source that can't be read leaves the game built here, alone.
    let out2 = d.path().join("scratch2");
    let gone = Folder(d.path().join("no-registry"));
    let b = build_registry(&gone, &out2, "maker/made-up", &side, Some(&local)).unwrap();
    assert_eq!(b.themes, vec!["me/my-theme@0.2.0"]);
}

#[test]
fn a_theme_is_served_under_a_name_a_registry_takes_and_drawn_by_its_look_id() {
    assert_eq!(registry_name("local/moss_night"), "local/moss-night");
    assert_eq!(registry_name("Me/My_Theme"), "me/my-theme");
    assert_eq!(registry_name("lex00/turnips"), "lex00/turnips");
    assert_eq!(registry_name("solo"), "local/solo");
    let d = tempfile::tempdir().unwrap();
    let root = theme_folder(d.path(), "local/moss_night", "0.1.0");
    fs::write(
        root.join("theme/look.txt"),
        "gp-look 1\nid moss-night\ntitle Moss Night\n",
    )
    .unwrap();
    let p = theme_pkg(&root).unwrap();
    assert_eq!(p.name, "local/moss-night");
    assert_eq!(p.renamed.as_deref(), Some("local/moss_night"));
    assert_eq!(Side::Folder(p).look_id(), "moss-night");
    // With no id in look.txt, the name's last part.
    let d2 = tempfile::tempdir().unwrap();
    let p2 = theme_pkg(&theme_folder(d2.path(), "me/my-theme", "0.2.0")).unwrap();
    assert_eq!(p2.renamed, None);
    assert_eq!(Side::Folder(p2).look_id(), "my-theme");
}

#[test]
fn the_original_side_is_the_game_in_its_own_pictures_with_no_theme_chosen() {
    let d = tempfile::tempdir().unwrap();
    let src = source(&d.path().join("src"));
    let side = against_side("original").unwrap();
    assert!(matches!(side, Side::Original));
    assert_eq!(side.tag(), "original");
    assert_eq!(side.content_param(), "");
    // Its registry holds the game and the themes offered, and nothing of
    // its own.
    let out = d.path().join("scratch");
    let b = build_registry(&src, &out, "maker/game", &side, None).unwrap();
    assert_eq!(b.themes[0], "the game's own look");
    let mut l = listed(&out);
    l.sort();
    assert!(l.iter().all(|(p, _)| p != "original"), "{l:?}");
    // The page chooses no theme.
    let page = d.path().join("page");
    fs::create_dir_all(&page).unwrap();
    fs::write(
        page.join("page.json"),
        r#"{"game":"maker/game","source":{"name":"r","url":"https://example.invalid/reg"},"content":"maker/default"}"#,
    )
    .unwrap();
    let ov = d.path().join("overlay");
    overlay(&ov, &page, &side, &BTreeMap::new()).unwrap();
    let pj: Value =
        serde_json::from_str(&fs::read_to_string(ov.join("page.json")).unwrap()).unwrap();
    assert!(pj.get("content").is_none(), "{pj}");
    // A folder's side still names its theme in the link.
    let root = theme_folder(d.path(), "me/my-theme", "0.2.0");
    let mine = Side::Folder(theme_pkg(&root).unwrap());
    assert_eq!(mine.content_param(), "&c=me/my-theme");
}
