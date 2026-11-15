//! The two journeys' steps from made-up facts, and copies found in a home
//! made here (Moss Hop's model and manifest; no ROM bytes).

use std::fs;

use super::*;

fn theme() -> ThemeFacts {
    ThemeFacts {
        name: "neon".into(),
        title: "Neon Night".into(),
        dir: PathBuf::from("/h/work/themes/neon"),
        from: Some(Origin {
            name: "moss-hop".into(),
            title: "Moss Hop".into(),
            kind: "game",
            dir: None,
        }),
        style: Some("night time, neon".into()),
        pictures: 4,
        to_draw: 0,
        changed_ms: 1000,
        package: Some("local/neon".into()),
        game_package: Some("lex00/moss-hop".into()),
        ..ThemeFacts::default()
    }
}

fn new_game() -> NewFacts {
    NewFacts {
        name: "my_hop".into(),
        dir: PathBuf::from("/h/work/mcp/my_hop"),
        from: Some(Origin {
            name: "mosshop".into(),
            title: "Moss Hop".into(),
            kind: "template",
            dir: None,
        }),
        assets: true,
        changed_ms: 1000,
        built_ms: Some(2000),
        ..NewFacts::default()
    }
}

fn of(f: Facts) -> Answer {
    answer(&f, &Around::default())
}

fn checked(problems: u64, at_ms: u64) -> Option<Check> {
    Some(Check {
        at_ms,
        problems,
        warnings: 2,
        first: (problems > 0).then(|| {
            "art/hero.png is 20 by 30, its rule draws 16 by 16 (theme/art.txt line 7)".into()
        }),
        all_to_draw: false,
    })
}

#[test]
fn a_theme_walks_its_six_steps_with_whose_turn_each_is() {
    // 1. No style yet: the person's.
    let mut f = theme();
    f.style = None;
    let a = of(Facts::Theme(f));
    assert_eq!((a.journey, a.stage.id.as_str()), ("theme", "T1"));
    assert_eq!(a.next.who, Who::Person);
    assert!(a.next.says.contains("style"), "{}", a.next.says);
    // 2. No pictures: the agent's, which asks before it spends.
    let mut f = theme();
    f.pictures = 0;
    let a = of(Facts::Theme(f));
    assert_eq!(a.stage.id, "T2");
    assert_eq!(a.next.who, Who::Agent);
    assert!(a.next.says.contains("costs money"), "{}", a.next.says);
    assert!(a.stands.contains("night time, neon"), "{}", a.stands);
    // 3. Not checked: the tools', behind the page's button.
    let a = of(Facts::Theme(theme()));
    assert_eq!(a.stage.id, "T3");
    assert_eq!(a.next.who, Who::Tools);
    assert_eq!(
        a.next.act,
        Some(Act::Start {
            action: "look-check"
        })
    );
    assert!(
        a.next
            .how
            .cli
            .as_deref()
            .unwrap()
            .starts_with("gp theme look check /h/work/themes/neon --game moss-hop")
    );
    // 3, with problems: the agent fixes the first, named with its line.
    let mut f = theme();
    f.check = checked(3, 1500);
    let a = of(Facts::Theme(f));
    assert_eq!((a.stage.id.as_str(), a.next.who), ("T3", Who::Agent));
    assert!(a.next.says.contains("art.txt line 7"), "{}", a.next.says);
    assert!(
        a.stands.contains("3 problems") && a.stands.contains("2 warnings"),
        "{}",
        a.stands
    );
    // A check older than the last change counts for nothing.
    let mut f = theme();
    f.check = checked(0, 500);
    assert_eq!(of(Facts::Theme(f)).next.who, Who::Tools);
    // 4. Passed: the person sees it in the game.
    let mut f = theme();
    f.check = checked(0, 1500);
    let a = of(Facts::Theme(f.clone()));
    assert_eq!((a.stage.id.as_str(), a.next.who), ("T4", Who::Person));
    assert_eq!(
        a.next.act,
        Some(Act::Start {
            action: "look-play"
        })
    );
    // 5. Seen: change pictures until the person says it's ready.
    f.played_ms = Some(1600);
    let a = of(Facts::Theme(f.clone()));
    assert_eq!((a.stage.id.as_str(), a.next.who), ("T5", Who::Person));
    assert!(a.next.says.contains("READY TO SHARE"));
    // 6. Ready: share, and the link and request are words, never sent.
    f.done = vec!["pictures".into()];
    let a = of(Facts::Theme(f));
    assert_eq!(a.stage.id, "T6");
    assert!(
        a.next.says.contains("Nothing is published"),
        "{}",
        a.next.says
    );
    assert_eq!(
        a.facts["share"]["link"],
        "https://nes-donkey-kong.intentius.workers.dev/p?g=lex00/moss-hop&c=local/neon"
    );
    assert!(
        a.facts["share"]["request"]["where"]
            .as_str()
            .unwrap()
            .contains("gp-registry")
    );
    // The steps list: done, now, later, with whose turn.
    let states: Vec<&str> = a.steps.iter().map(|s| s.state).collect();
    assert_eq!(states, ["done", "done", "done", "done", "done", "now"]);
    assert_eq!(a.steps[2].who, Who::Tools);
    assert_eq!(a.stage.of, 6);
}

#[test]
fn a_new_game_walks_its_steps() {
    // Pictures and levels not made: the agent's.
    let mut f = new_game();
    f.assets = false;
    let a = of(Facts::New(f));
    assert_eq!(
        (a.journey, a.stage.id.as_str(), a.next.who),
        ("new", "N3", Who::Agent)
    );
    // Not built: the tools build it behind BUILD IT TO PLAY.
    let mut f = new_game();
    f.built_ms = None;
    let a = of(Facts::New(f));
    assert_eq!((a.stage.id.as_str(), a.next.who), ("N3", Who::Tools));
    assert_eq!(a.next.act, Some(Act::Start { action: "build" }));
    // Built: the stage is the person's to change, with the agent.
    let a = of(Facts::New(new_game()));
    assert_eq!((a.stage.id.as_str(), a.next.who), ("N4", Who::Person));
    let states: Vec<&str> = a.steps.iter().map(|s| s.state).collect();
    assert_eq!(states, ["done", "done", "done", "now", "later", "later"]);
    // An edit not checked: the tools check it, and the person's own step
    // comes first among the others.
    let mut f = new_game();
    f.edits = 2;
    f.edited_ms = 3000;
    f.changed_ms = 3000;
    let a = of(Facts::New(f.clone()));
    assert_eq!(a.next.who, Who::Tools);
    assert_eq!(a.next.act, Some(Act::Start { action: "verify" }));
    assert_eq!(a.also[0].who, Who::Person);
    assert!(
        a.also
            .iter()
            .any(|s| s.says.contains("BUILD IT TO PLAY again"))
    );
    // A stage gp level verify refused to check: no CHECK THE STAGE as
    // the next step, the person's step is next.
    let mut g = f.clone();
    g.no_check = Some("a made-up refusal of a made-up stage".into());
    let a = of(Facts::New(g));
    assert_eq!(a.next.who, Who::Person);
    assert!(a.next.act.is_none());
    assert!(!a.also.iter().any(|s| s.says.contains("CHECK THE STAGE")));
    // The check failed after the edit: the agent fixes it, or UNDO.
    f.verify = Some((3500, false, "x\nit could NOT be finished".into()));
    let a = of(Facts::New(f));
    assert_eq!(a.next.who, Who::Agent);
    assert!(
        a.next.says.contains("UNDO") && a.next.says.contains("could NOT be finished"),
        "{}",
        a.next.says
    );
    // The person says the stage is done: a look; then play, and a share
    // link made into their own registry, with what it takes for others.
    let mut f = new_game();
    f.done = vec!["stage".into()];
    let a = of(Facts::New(f.clone()));
    assert_eq!(a.stage.id, "N5");
    assert!(a.next.says.contains("MAKE A THEME FOR IT"));
    f.done.push("look".into());
    let a = of(Facts::New(f));
    assert_eq!(a.stage.id, "N6");
    assert!(
        a.next.says.contains("MAKE A SHARE LINK")
            && a.next.says.contains("have to be put online")
            && !a.next.says.contains("isn't built yet"),
        "{}",
        a.next.says
    );
    assert!(
        a.next
            .how
            .cli
            .as_deref()
            .is_some_and(|c| c.starts_with("gp game share ")),
        "{:?}",
        a.next.how.cli
    );
}

#[test]
fn a_picture_waiting_on_the_person_comes_first() {
    let mut f = theme();
    f.check = checked(0, 1500);
    f.played_ms = Some(1600);
    let ar = Around {
        pictures: vec![
            json!({"id": "p1", "state": "open", "place": 1, "picture": "art/hero.png",
            "why": "A red cap.", "game": "moss-hop"}),
        ],
        ..Around::default()
    };
    let a = answer(&Facts::Theme(f), &ar);
    assert_eq!(a.next.who, Who::Person);
    assert!(a.next.says.contains("ACCEPT or REJECT"));
    assert_eq!(a.waits_for_yes[0].what, "picture");
    assert_eq!(a.version, "next 2");
}

fn mosshop(f: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../templates/mosshop")
            .join(f),
    )
    .unwrap()
}

fn put(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

#[test]
fn copies_are_found_where_the_tools_make_them() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let home = root.join("home");
    let games = root.join("games");
    let paths = gp_evidence::Paths {
        home: home.clone(),
        store: home.join("store"),
        roms: root.join("roms"),
    };
    // A game of the games checkout, titled by its manifest.
    put(
        &games.join("moss-hop/package.toml"),
        "[package]\nname = \"lex00/moss-hop\"\n",
    );
    put(&games.join("moss-hop/model.toml"), &mosshop("model.toml"));
    put(
        &games.join("moss-hop/template.toml"),
        "[template]\nid = \"moss-hop\"\ntitle = \"Moss Hop\"\n",
    );
    // A theme as gp theme look new writes it, with no record: its game
    // is read from its package's first line.
    let t = home.join("work/themes/neon");
    put(
        &t.join("package.toml"),
        "# local/neon: Neon, a large-art theme for Moss Hop.\n[package]\nname = \"local/neon\"\n",
    );
    put(
        &t.join("theme/look.txt"),
        "gp-look 1\nid neon\ntitle Neon\nart art.txt\n",
    );
    put(&t.join("theme/art.txt"), "gp-art 1\n# TO DRAW hero\n");
    // A folder THEMES writes for a game is no copy.
    put(
        &home.join("work/themes/moss-hop/results/x/check.json"),
        "{}",
    );
    // A new game, as template_fork leaves it.
    let n = home.join("work/mcp/my_hop");
    put(&n.join("model.toml"), &mosshop("model.toml"));
    put(&n.join("template.toml"), &mosshop("template.toml"));
    put(&home.join("work/mcp/.store/x"), "");

    let all = list(&home, Some(&games));
    let names: Vec<String> = all
        .iter()
        .map(|c| format!("{}/{}", c.kind.word(), c.name))
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"theme/neon".into()) && names.contains(&"new/my_hop".into()));
    let neon = find(&home, Some(&games), "neon").unwrap();
    let from = neon.from.clone().unwrap();
    assert_eq!(
        (from.name.as_str(), from.kind, from.title.as_str()),
        ("moss-hop", "game", "Moss Hop")
    );
    let hop = find(&home, Some(&games), "new/my_hop").unwrap();
    assert_eq!(hop.from.as_ref().unwrap().title, "Moss Hop");
    assert!(
        find(&home, Some(&games), "nothing")
            .unwrap_err()
            .to_string()
            .contains("theme/neon")
    );

    // No style: step 1, the person's. The style words come from the
    // journey record the page writes.
    let a = answer(&gather(&paths, &all, &neon), &Around::default());
    assert_eq!(a.stage.id, "T1");
    write(
        &home,
        Kind::Theme,
        "neon",
        "journey.json",
        &json!({"from": "moss-hop", "style": "neon"}),
    )
    .unwrap();
    let neon = find(&home, Some(&games), "neon").unwrap();
    let f = gather(&paths, &all, &neon);
    let a = answer(&f, &Around::default());
    assert_eq!(a.stage.id, "T2", "{}", a.stands);
    // A picture, then a check left by gp theme look check.
    put(&t.join("theme/art/hero.png"), "not really a png");
    note_check(
        &home,
        &t.join("theme"),
        &json!({"ok": false, "problems": 1, "warnings": 0,
        "findings": [{"group": "kit", "level": "problem", "file": "theme/art.txt", "line": 2,
                      "message": "still to draw: the hero"}]}),
    );
    let neon = find(&home, Some(&games), "neon").unwrap();
    let a = answer(&gather(&paths, &all, &neon), &Around::default());
    assert_eq!(
        (a.stage.id.as_str(), a.next.who),
        ("T3", Who::Agent),
        "{}",
        a.next.says
    );
    assert!(
        a.next.says.contains("1 picture the game still needs"),
        "{}",
        a.next.says
    );

    // The new game: its assets aren't lifted here, so the agent's.
    let a = answer(&gather(&paths, &all, &hop), &Around::default());
    assert_eq!((a.stage.id.as_str(), a.next.who), ("N3", Who::Agent));

    // The copy opened last answers when it was opened after the last game.
    assert_eq!(last_copy(&home), None);
    remember(&home, Kind::New, "my_hop");
    assert_eq!(last_copy(&home).as_deref(), Some("new/my_hop"));
    let a = crate::next::copy_answer(&paths, Some(&games), None, None)
        .unwrap()
        .unwrap();
    assert_eq!(a.subject.as_deref(), Some("my_hop"));
    assert_eq!(a.copies.len(), 2);
    let text = crate::next::text_of(&a);
    assert!(
        text.starts_with("your new game my_hop, step 3 of 6: See it play"),
        "{text}"
    );
    assert!(
        text.contains("theme neon (from Moss Hop): step 3 of 6"),
        "{text}"
    );
    // A game named answers for the game, never a copy.
    assert!(
        crate::next::copy_answer(&paths, Some(&games), Some("moss-hop"), None)
            .unwrap()
            .is_none()
    );
    // A copy named that isn't there is refused in words.
    assert!(crate::next::copy_answer(&paths, Some(&games), None, Some("gone")).is_err());
}

#[test]
fn a_command_on_a_folder_elsewhere_leaves_no_record() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let elsewhere = d.path().join("my-theme");
    put(&elsewhere.join("theme/look.txt"), "gp-look 1\n");
    note_check(&home, &elsewhere, &json!({"ok": true, "problems": 0}));
    assert!(!home.join("work/journeys").exists());
}
