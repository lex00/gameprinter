//! The loop's stages read from fixture folders (a games checkout, a home
//! and a ROM folder in a temporary folder, Moss Hop's model and mapping and
//! a made-up ROM image: no ROM bytes in the repo), and the answer's shape.

use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::progress_measure::{FirstSplit, Measured};

/// A games checkout, a home and a ROM folder.
struct Fx {
    _d: tempfile::TempDir,
    root: PathBuf,
    paths: gp_evidence::Paths,
}

fn mosshop(f: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../templates/mosshop")
            .join(f),
    )
    .unwrap()
}

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

impl Fx {
    fn new() -> Fx {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().to_owned();
        for s in ["games", "home", "roms"] {
            fs::create_dir_all(root.join(s)).unwrap();
        }
        let paths = gp_evidence::Paths {
            home: root.join("home"),
            store: root.join("home/store"),
            roms: root.join("roms"),
        };
        Fx { _d: d, root, paths }
    }

    fn games(&self) -> PathBuf {
        self.root.join("games")
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    /// A made-up NES image in the ROM folder; its headerless hash.
    fn rom(&self, name: &str, fill: u8) -> String {
        let mut rom = b"NES\x1a\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        rom.extend(vec![fill; 16384 + 8192]);
        let p = self.root.join("roms/nes").join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, &rom).unwrap();
        crate::template::sha256_hex(&rom[16..])
    }

    /// A game folder `g` with Moss Hop's model and mapping (its player
    /// mapped), a ROM it accepts, and the given corpus clips.
    fn game(&self, g: &str, clips: &[&str]) -> PathBuf {
        let sha = self.rom(&format!("{g}.nes"), 7);
        let d = self.games().join(g);
        write(&d.join("package.toml"), "");
        write(
            &d.join("template.toml"),
            &format!(
                "[template]\ntitle = \"Test Game\"\n\n[source]\nid = \"{g}\"\nplatform = \"nes\"\naccepts = [\"{sha}\"]\n"
            ),
        );
        write(&d.join("model.toml"), &mosshop("model.toml"));
        write(&d.join("mapping.toml"), &mosshop("mapping.toml"));
        if !clips.is_empty() {
            let mut plan = "[corpus]\nformat = \"1.0.0\"\nname = \"t\"\nversion = \"1.0.0\"\nrecipes = \"R\"\n\n[target]\ngame = \"t\"\nrom = \"aa\"\n\n[needs]\nmapping = \"22\"\n".to_owned();
            for (i, id) in clips.iter().enumerate() {
                plan.push_str(&format!("\n[[clip]]\nid = \"{id}\"\nname = \"c{i}\"\nsource = \"played\"\nstart = \"inputs/c{i}.start.presses\"\npresses = \"inputs/c{i}.presses\"\nframes = 10\n"));
                write(
                    &d.join(format!("inputs/c{i}.start.presses")),
                    "gp-presses 0\nframes 4\nstart@1-2\n",
                );
                write(
                    &d.join(format!("inputs/c{i}.presses")),
                    "gp-presses 0\nframes 10\nright@0-5\n",
                );
            }
            write(&d.join("corpus.toml"), &plan);
        }
        d
    }

    /// A session clip of the game's mechanic `m`.
    fn session_clip(&self, g: &str, m: &str, n: u64) {
        write(
            &self
                .home()
                .join(format!("mechanics/{g}/{m}/clips/{n:03}.toml")),
            "start = 0\nframes = 10\n",
        );
        write(
            &self.home().join(format!("mechanics/{g}/{m}/session.toml")),
            "",
        );
    }

    fn queue(&self, g: &str, q: Value) {
        write(
            &self.home().join(format!("mechanics/{g}/queue.json")),
            &q.to_string(),
        );
    }

    /// The gate's scores: `exact` of the clips meet the bars, on the
    /// game's model as it stands (or an older one).
    fn gate(&self, g: &str, ids: &[&str], exact: usize, stale: bool) {
        let d = self.games().join(g);
        let mut b = fs::read(d.join("model.toml")).unwrap();
        b.extend(fs::read(d.join("mapping.toml")).unwrap());
        let hash = if stale {
            "0".repeat(64)
        } else {
            gp_evidence::Hash::of(&b).to_string()
        };
        let clips: Vec<Value> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let ok = i < exact;
                json!({"id": id, "mechanic": "walk", "clip": i + 1,
                       "straight": {"together": if ok { 10 } else { 4 }, "of": 10},
                       "first_split": if ok { Value::Null } else { json!({"frame": 9, "clip_frame": 4, "what": "x apart"}) },
                       "bars": {"together": ok, "longest_stretch": null}})
            })
            .collect();
        write(
            &self.home().join(format!("work/refine/{g}/status.json")),
            &json!({"model": {"commit": "abc", "hash": hash}, "clips": clips, "rows": []})
                .to_string(),
        );
    }

    /// The whole-game measure, on the model as it stands.
    fn whole(&self, g: &str, first: Option<u64>) {
        let d = self.games().join(g);
        let m = Measured {
            version: crate::progress::VERSION.into(),
            model: crate::progress_measure::model_key(&d),
            whole: Some(crate::progress_measure::Whole {
                from: 4,
                compared: 1000,
                first: first.map(|at| FirstSplit {
                    at,
                    frame: at,
                    original_frame: at + 4,
                    level: "1-1".into(),
                    paths: vec![],
                }),
                ..Default::default()
            }),
            ..Measured::default()
        };
        write(
            &self.home().join(format!("work/progress/{g}/{g}.json")),
            &serde_json::to_string(&m).unwrap(),
        );
    }

    fn facts(&self, g: Option<&str>) -> Facts {
        gather(&self.paths, Some(&self.games()), g, None).unwrap()
    }

    fn answer(&self, g: Option<&str>) -> Answer {
        answer(&self.facts(g))
    }
}

#[test]
fn p1_no_game_and_no_rom_asks_for_the_file() {
    let fx = Fx::new();
    let a = fx.answer(None);
    assert_eq!((a.stage.n, a.stage.id.as_str()), (1, "P1"));
    assert_eq!(a.stands, "No game file is on this computer yet.");
    assert_eq!(a.next.who, Who::Person);
    assert!(
        a.next
            .says
            .starts_with("Tell your agent where your copy of the game is"),
        "{}",
        a.next.says
    );
    assert_eq!(a.subject, None);
}

#[test]
fn p2_a_rom_and_no_game_sets_it_up_or_asks_which() {
    let fx = Fx::new();
    fx.rom("Kong.nes", 1);
    let a = fx.answer(None);
    assert_eq!(a.stage.n, 2);
    assert_eq!(
        a.stands,
        "Your game file Kong.nes is here, but no game is set up from it yet."
    );
    assert_eq!(a.next.who, Who::Agent);
    fx.rom("Other.nes", 2);
    let a = fx.answer(None);
    assert_eq!(a.next.who, Who::Person);
    assert_eq!(
        a.next.says,
        "Say which of them to bring over: Kong.nes and Other.nes."
    );
}

#[test]
fn p0_two_games_and_none_opened_asks_which() {
    let fx = Fx::new();
    fx.game("a", &[]);
    fx.game("b", &[]);
    let a = fx.answer(None);
    assert_eq!(a.stage.n, 0);
    assert_eq!(a.next.says, "Say which game to work on: a and b.");
    // The page's last game is the subject.
    write(&fx.home().join("work/serve/last-game"), "b\n");
    assert_eq!(fx.answer(None).subject.as_deref(), Some("b"));
    assert!(gather(&fx.paths, Some(&fx.games()), Some("c"), None).is_err());
}

#[test]
fn p3_the_player_not_mapped() {
    let fx = Fx::new();
    let d = fx.game("t", &[]);
    let m = fs::read_to_string(d.join("mapping.toml")).unwrap();
    fs::write(
        d.join("mapping.toml"),
        m.replace("entities.player[", "entities.goal["),
    )
    .unwrap();
    let f = fx.facts(Some("t"));
    assert!(f.rom_found && !f.player_mapped);
    let a = answer(&f);
    assert_eq!(a.stage.n, 3);
    assert_eq!(a.next.who, Who::Agent);
    assert!(
        a.next
            .says
            .starts_with("Your agent can find where the game keeps the player")
    );
}

#[test]
fn p4_no_clips_sends_the_tools_first() {
    let fx = Fx::new();
    fx.game("t", &[]);
    let a = fx.answer(Some("t"));
    assert_eq!(a.stage.n, 4);
    assert_eq!(a.next.who, Who::Tools);
    assert_eq!(a.next.how.cli.as_deref(), Some("gp mechanic triage t"));
    assert_eq!(
        a.stands,
        "The tools know where the player is in Test Game. No play is recorded yet."
    );
}

#[test]
fn a_missing_rom_comes_first() {
    let fx = Fx::new();
    fx.game("t", &[]);
    fs::remove_file(fx.root.join("roms/nes/t.nes")).unwrap();
    let a = fx.answer(Some("t"));
    assert_eq!(a.next.who, Who::Person);
    assert!(
        a.next
            .says
            .starts_with("Game Printer can't find your copy of Test Game")
    );
}

#[test]
fn p5_session_clips_not_in_the_tests() {
    let fx = Fx::new();
    fx.game("t", &["t.walk.001"]);
    fx.session_clip("t", "walk", 1);
    fx.session_clip("t", "walk", 2);
    // A sealed run never goes in.
    fx.session_clip("t", "sealed-run", 1);
    let f = fx.facts(Some("t"));
    assert_eq!(f.not_kept, ["walk 2"]);
    assert_eq!((f.session_clips, f.corpus_clips), (3, 1));
    let a = answer(&f);
    assert_eq!(a.stage.n, 5);
    assert_eq!(
        a.next.how.cli.as_deref(),
        Some("gp corpus import t --write")
    );
}

#[test]
fn p6_scores_from_an_older_model_or_none() {
    let fx = Fx::new();
    fx.game("t", &["t.walk.001", "t.walk.002"]);
    let a = fx.answer(Some("t"));
    assert_eq!(a.stage.n, 6);
    assert_eq!(a.next.who, Who::Tools);
    assert!(
        a.next.says.contains("Press START THE WORK on REFINE"),
        "{}",
        a.next.says
    );
    fx.gate("t", &["t.walk.001", "t.walk.002"], 1, true);
    let a = fx.answer(Some("t"));
    assert_eq!(a.stage.n, 6);
    assert!(
        a.stands
            .starts_with("The model of Test Game changed since its clips were last scored"),
        "{}",
        a.stands
    );
    // Scoring now: nothing to start, and when to look again.
    let mut f = fx.facts(Some("t"));
    f.background.on = true;
    let a = answer(&f);
    assert_eq!(a.running.len(), 1);
    assert!(!a.next.says.contains("START"), "{}", a.next.says);
}

#[test]
fn p7_and_p8_some_clips_differ() {
    let fx = Fx::new();
    fx.game("t", &["t.walk.001", "t.walk.002"]);
    fx.gate("t", &["t.walk.001", "t.walk.002"], 1, false);
    let a = fx.answer(Some("t"));
    assert_eq!(a.stage.n, 7);
    assert_eq!(
        a.stands,
        "1 of 2 clips of Test Game play the same on the model as on the original."
    );
    assert_eq!(a.next.who, Who::Agent);
    assert!(
        a.next.says.contains("t.walk.002 (at its frame 4)"),
        "{}",
        a.next.says
    );
    // A row only the person can play: the agent's work stays next, the
    // person's play comes first among the others.
    fx.queue(
        "t",
        json!([{"mechanic": "walk", "status": "next"},
               {"mechanic": "die", "status": "next", "triage": {"by": "you", "why": "no tool got it",
                "request": {"where": "level 4", "do": "lose a life at the far end", "stop": "the next life starts"}}}]),
    );
    let a = fx.answer(Some("t"));
    assert_eq!(a.stage.n, 8);
    assert!(
        a.stands
            .ends_with("1 quest item needs you: play the tools couldn't get."),
        "{}",
        a.stands
    );
    assert_eq!(a.also[0].who, Who::Person);
    assert_eq!(
        a.also[0].says,
        "Press PLAY FROM HERE on die. Lose a life at the far end. Stop: The next life starts."
    );
    assert_eq!(a.also[0].act, Some(Act::PlayFrom { row: "die".into() }));
}

#[test]
fn p9_to_p12_the_full_run_and_online() {
    let fx = Fx::new();
    fx.game("t", &["t.walk.001"]);
    fx.gate("t", &["t.walk.001"], 1, false);
    let a = fx.answer(Some("t"));
    assert_eq!(a.stage.n, 9);
    assert_eq!(a.next.who, Who::Tools);
    assert_eq!(
        a.next.how.cli.as_deref(),
        Some("gp progress --template t --run t --measure")
    );
    fx.whole("t", Some(300));
    let a = fx.answer(Some("t"));
    assert_eq!(a.stage.n, 10);
    assert!(a.stands.contains("300 of 1000 frames"), "{}", a.stands);
    fx.whole("t", None);
    let a = fx.answer(Some("t"));
    assert_eq!(a.stage.n, 11);
    assert_eq!(a.waits_for_yes.len(), 1);
    assert_eq!(a.next.who, Who::Person);
    assert!(a.next.says.starts_with("Say yes"));
    write(
        &fx.home().join("work/published/t.json"),
        r#"{"link": "https://example.org/t"}"#,
    );
    let a = fx.answer(Some("t"));
    assert_eq!(a.stage.n, 12);
    assert_eq!(a.stands, "Test Game is online at https://example.org/t.");
    assert!(a.waits_for_yes.is_empty());
    // A change to the model since the measure: measure again.
    let d = fx.games().join("t");
    let m = fs::read_to_string(d.join("model.toml")).unwrap();
    fs::write(d.join("model.toml"), format!("{m}\n# changed\n")).unwrap();
    fx.gate("t", &["t.walk.001"], 1, false);
    let f = fx.facts(Some("t"));
    assert_eq!(f.whole, Whole::Stale);
}

#[test]
fn a_hand_off_and_a_question_from_the_agent() {
    let fx = Fx::new();
    fx.game("t", &["t.walk.001"]);
    fx.gate("t", &["t.walk.001"], 0, false);
    fx.queue(
        "t",
        json!([{"mechanic": "walk", "status": "agent working"}, {"mechanic": "jump", "status": "asking you"}]),
    );
    write(
        &fx.home().join("mechanics/t/walk/talk.ndjson"),
        "{\"by\":\"player\",\"text\":\"Let's do walk\",\"n\":1}\n",
    );
    write(
        &fx.home().join("mechanics/t/jump/talk.ndjson"),
        "{\"by\":\"agent\",\"text\":\"Does the jump go higher when you hold A?\",\"n\":1}\n",
    );
    let a = fx.answer(Some("t"));
    // The person's answer first: the tools wait on it.
    assert_eq!(a.next.who, Who::Person);
    assert_eq!(
        a.next.says,
        "Your agent asked about jump: \"Does the jump go higher when you hold A?\" Answer it on CLIPS, over the game."
    );
    assert_eq!(a.also[0].who, Who::Agent);
    assert_eq!(
        a.also[0].says,
        "Your agent can answer your hand-off on walk. If it hasn't yet, tell it you handed it over."
    );
}

#[test]
fn clues_from_the_page_and_the_files() {
    let fx = Fx::new();
    fx.game("t", &["t.walk.001"]);
    fx.gate("t", &["t.walk.001"], 0, false);
    fx.queue(
        "t",
        json!([{"mechanic": "die", "status": "next", "triage": {"by": "you", "request": {"do": "lose a life", "stop": "the next life starts."}}}]),
    );
    let mut f = fx.facts(Some("t"));
    let sig = |s: &str, shown| PageSignal {
        signal: s.into(),
        row: Some("die".into()),
        shown,
    };
    f.page = Some(sig("idle", 0));
    assert_eq!(
        answer(&f).clue.unwrap().says,
        "One bit of play would help: die. PLAY FROM HERE puts you at the spot."
    );
    f.page = Some(sig("idle", 3));
    assert!(answer(&f).clue.unwrap().says.contains("HAND TO THE AGENT"));
    f.page = Some(sig("drops", 0));
    assert_eq!(
        answer(&f).clue.unwrap().says,
        "The clip can be short. You only need to reach the stop: the next life starts. F5 saves a spot, F8 goes back to it."
    );
    f.page = Some(sig("hops", 0));
    let a = answer(&f);
    assert_eq!(
        a.clue.unwrap().says,
        format!("Looking for where to start? Right now: {}", a.next.says)
    );
    // Paused work the person may not know about, on the page only.
    f.page = None;
    f.scores.as_mut().unwrap().stale = true;
    assert_eq!(answer(&f).clue, None);
    f.background.page_open = true;
    assert_eq!(answer(&f).clue.unwrap().signal, "paused");
    // Words that show the person expects a ROM or an install.
    f.background.page_open = false;
    f.notes = vec!["where do I download the new rom?".into()];
    assert!(
        answer(&f)
            .clue
            .unwrap()
            .says
            .contains("never writes or changes a ROM file")
    );
}

#[test]
fn the_json_has_the_answers_shape() {
    let fx = Fx::new();
    fx.game("t", &[]);
    let v = json_of(&fx.answer(Some("t")));
    assert_eq!(v["version"], VERSION);
    assert_eq!(v["journey"], "port");
    assert_eq!(v["subject"], "t");
    assert_eq!(
        v["stage"],
        json!({"id": "P4", "n": 4, "of": 12, "name": "First clips"})
    );
    assert_eq!(v["next"]["who"], "tools");
    for k in ["stands", "text"] {
        assert!(v[k].is_string(), "{k}");
    }
    for k in ["also", "running", "waits_for_yes"] {
        assert!(v[k].is_array(), "{k}");
    }
    assert!(v["next"]["how"].as_object().unwrap().contains_key("mcp"));
    assert!(v["clue"].is_null());
    assert_eq!(v["facts"]["player_mapped"], true);
    assert!(v["text"].as_str().unwrap().contains("Next (the tools): "));
}

#[test]
fn words_are_cut_at_a_space() {
    assert_eq!(cut("one two three", 9), "one two...");
    assert_eq!(cut("short", 9), "short");
    assert_eq!(sentence("lose a life"), "Lose a life.");
    assert_eq!(
        few(&["a", "b", "c", "d"].map(String::from)),
        "a, b and 2 more"
    );
}
