//! The sound tools on the made-up game (crate::testgame) with a made-up
//! song: describe, the readiness gate, edits with clips, history and undo,
//! and the in-context preview. No game data.

use std::fs;

use serde_json::{Value, json};

use crate::{Config, tools};
use gp_sound::apu::Channel;
use gp_sound::song::*;

struct Fixture {
    _dir: tempfile::TempDir,
    config: Config,
}

fn song(name: &str, looped: bool) -> Song {
    let tempo = Tempo {
        frames_per_row: 8,
        rows_per_pattern: 4,
    };
    let n = |start: u64, pitch: u8, instrument: u16| Placed {
        start,
        note: Note {
            at: 0,
            len: 8,
            pitch,
            fine: 0,
            instrument,
        },
    };
    Song {
        name: name.into(),
        kind: Kind::Music,
        origin: Origin::Log,
        confidence: 0.95,
        driver: None,
        frames: 32,
        loop_frame: looped.then_some(0),
        tempo,
        tracks: vec![
            Track::from_placed(
                Channel::Pulse1,
                &[n(0, 72, 0), n(8, 76, 0), n(16, 79, 0)],
                tempo,
                32,
            ),
            Track::from_placed(Channel::Triangle, &[n(0, 48, 1), n(16, 43, 1)], tempo, 32),
        ],
        request: None,
        event: None,
        level: Some("main".into()),
        state: None,
        captured: None,
        notes: vec![],
    }
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (config, _) = crate::testgame::config(dir.path());
        let model = config.work.join("copy/model.toml");
        let text = fs::read_to_string(&model)
            .unwrap()
            .replace("schema = \"0.4.0\"", "schema = \"0.4.3\"")
            .replace(
                "[assets.level]",
                "[assets.music]\nkind = \"songs\"\nrole = \"music.level\"\ncapture = { recording = \"made\", frames = \"1..\", name = \"tune\" }\n\n[assets.level]",
            )
            + "\n[present.sound]\nmusic = [{ slot = \"music\", song = \"theme\", level = \"main\" }]\n";
        fs::write(&model, text).unwrap();
        let mut set = SongSet::new(gp_sound::apu::Region::Ntsc);
        for (name, voice, role) in [
            ("lead", Voice::Pulse, "lead"),
            ("bass", Voice::Triangle, "bass"),
        ] {
            set.instruments.push(Instrument {
                name: name.into(),
                voice,
                role: Some(role.into()),
                volume: vec![12, 10, 8],
                duty: if voice == Voice::Pulse {
                    vec![2]
                } else {
                    vec![]
                },
                pitch: vec![],
                sample: None,
                notes: 0,
            });
        }
        set.songs.push(song("theme", true));
        let mut unlooped = song("sketch", false);
        unlooped.level = None;
        set.songs.push(unlooped);
        let d = config.home.join("work/sound/made0000/tune");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("songs.json"), set.to_text()).unwrap();
        Self { _dir: dir, config }
    }

    fn call(&self, tool: &str, args: Value) -> (Value, bool) {
        let r = tools::call(&self.config, tool, args).expect("a listed tool");
        (r.json, r.is_error)
    }
}

#[test]
fn describe_says_what_plays_and_what_is_ready() {
    let f = Fixture::new();
    let (v, err) = f.call("sound_describe", json!({"copy": "copy"}));
    assert!(!err, "{v}");
    let songs = &v["slots"][0]["songs"];
    assert_eq!(songs[0]["name"], "theme");
    assert_eq!(songs[0]["editable"], true, "{v}");
    assert_eq!(songs[0]["bound_in_model"][0]["level"], "main");
    assert_eq!(songs[0]["instruments"][0]["plays_through"], "apu");
    assert_eq!(songs[1]["editable"], false);
    let (b, err) = f.call("sound_banks", json!({}));
    assert!(!err, "{b}");
    assert_eq!(b["banks"][0]["name"], "apu");
    assert!(b["banks"][0]["patches"][0]["preview"].is_string());
}

#[test]
fn edits_write_clips_history_and_undo_and_respect_readiness() {
    let f = Fixture::new();
    let (v, err) = f.call(
        "sound_edit",
        json!({"copy": "copy", "ops": [
            {"op": "remap", "role": "bass", "target": {"kind": "gm", "program": 33}},
            {"op": "transpose", "song": "theme", "semitones": -12, "channel": "pulse1"},
        ], "prompt": "a lower lead", "write": true, "confirmed_by_person": true}),
    );
    assert!(!err, "{v}");
    assert_eq!(v["written"], true);
    assert!(fs::metadata(v["before"].as_str().unwrap()).unwrap().len() > 1000);
    assert!(fs::metadata(v["after"].as_str().unwrap()).unwrap().len() > 1000);
    let copy = f.config.work.join("copy");
    let set =
        SongSet::parse(&fs::read_to_string(copy.join("sound/music.songs.json")).unwrap()).unwrap();
    let p = set
        .song("theme")
        .unwrap()
        .track(Channel::Pulse1)
        .unwrap()
        .placed(Tempo {
            frames_per_row: 8,
            rows_per_pattern: 4,
        });
    assert_eq!(p[0].note.pitch, 60);
    assert!(
        fs::read_to_string(copy.join("sound/banks.toml"))
            .unwrap()
            .contains("program = 33")
    );
    // The unlooped song isn't ready, so an op naming it is refused.
    let (r, err) = f.call(
        "sound_edit",
        json!({"copy": "copy", "ops": [{"op": "tempo", "song": "sketch", "frames_per_row": 4}]}),
    );
    assert!(err, "{r}");
    assert_eq!(r["stage"], "readiness");
    // Out of range is refused, not clamped.
    let (_, err) = f.call(
        "sound_edit",
        json!({"copy": "copy", "ops": [{"op": "transpose", "song": "theme", "semitones": 100}]}),
    );
    assert!(err);
    // Undo changes the person's sound at once too, so it needs their yes.
    let (u, err) = f.call("sound_edit", json!({"copy": "copy", "undo": true}));
    assert!(
        err && u["refused"]
            .as_str()
            .unwrap()
            .contains("confirmed_by_person"),
        "{u}"
    );
    let (u, err) = f.call(
        "sound_edit",
        json!({"copy": "copy", "undo": true, "confirmed_by_person": true}),
    );
    assert!(!err, "{u}");
    assert_eq!(u["undone"], 1);
    assert!(!copy.join("sound/music.songs.json").exists());
    assert!(!copy.join("sound/banks.toml").exists());
}

#[test]
fn the_preview_cues_the_level_song_in_context() {
    let f = Fixture::new();
    let (v, err) = f.call("sound_preview", json!({"copy": "copy", "baseline": "one"}));
    assert!(!err, "{v}");
    assert_eq!(v["cued"][0]["music"], "music/theme");
    assert!(v["seconds"].as_f64().unwrap() > 0.0);
    assert!(fs::metadata(v["file"].as_str().unwrap()).unwrap().len() > 1000);
}

#[test]
fn an_edit_is_proposed_and_only_the_person_s_answer_changes_the_copy() {
    use crate::sound_proposal::{self as sp, Answer};
    let f = Fixture::new();
    let copy = f.config.work.join("copy");
    let home = &f.config.home;
    let op = json!([{"op": "transpose", "song": "theme", "semitones": -12, "channel": "pulse1"}]);
    // A proposal needs one sentence for the person.
    let (r, err) = f.call("sound_edit", json!({"copy": "copy", "ops": op}));
    assert!(err && r["refused"].as_str().unwrap().contains("why"), "{r}");
    // A direct write without the person's yes is refused and writes nothing.
    let (r, err) = f.call(
        "sound_edit",
        json!({"copy": "copy", "ops": op, "write": true}),
    );
    assert!(err && r["refused"].as_str().unwrap().contains("yes"), "{r}");
    assert!(!copy.join("sound/music.songs.json").exists());
    // The default proposes: the clips and the change are kept apart from
    // the copy, which is unchanged.
    let (v, err) = f.call(
        "sound_edit",
        json!({"copy": "copy", "ops": op, "why": "A lower lead, as you asked."}),
    );
    assert!(!err, "{v}");
    assert_eq!(v["proposed"], true);
    assert_eq!(v["written"], false);
    let id = v["id"].as_str().unwrap().to_owned();
    assert!(!copy.join("sound/music.songs.json").exists());
    assert!(!copy.join(crate::sound::HISTORY_FILE).exists());
    let old = sp::wav(home, &id, "old").unwrap();
    let new = sp::wav(home, &id, "new").unwrap();
    assert!(old.starts_with(b"RIFF") && new.starts_with(b"RIFF") && old != new);
    assert!(sp::wav(home, &id, "../old").is_none());
    assert_eq!(sp::for_next(home)[0]["place"], 1);
    // A second one queues behind it; REJECT keeps the copy as it was and
    // carries the words.
    let (two, _) = f.call(
        "sound_edit",
        json!({"copy": "copy", "ops": [{"op": "tempo", "song": "theme", "frames_per_row": 6}], "why": "Faster."}),
    );
    let id2 = two["id"].as_str().unwrap().to_owned();
    assert_eq!(sp::queue(home).len(), 2);
    let r = sp::decide(home, &id2, Answer::Reject, Some("  keep the speed  ")).unwrap();
    assert_eq!(r["words"], "keep the speed");
    assert!(!copy.join("sound/music.songs.json").exists());
    // ACCEPT writes it, into the history too.
    let a = sp::decide(home, &id, Answer::Accept, None).unwrap();
    assert_eq!(a["state"], "accepted");
    let set =
        SongSet::parse(&fs::read_to_string(copy.join("sound/music.songs.json")).unwrap()).unwrap();
    let tempo = Tempo {
        frames_per_row: 8,
        rows_per_pattern: 4,
    };
    let lead = |s: &SongSet| {
        s.song("theme")
            .unwrap()
            .track(Channel::Pulse1)
            .unwrap()
            .placed(tempo)[0]
            .note
            .pitch
    };
    assert_eq!(lead(&set), 60);
    assert!(
        sp::decide(home, &id, Answer::Reject, None).is_err(),
        "answered once"
    );
    // The agent reads both answers once.
    let (l, text) = sp::list(home, None, false);
    assert_eq!(l["answered"].as_array().unwrap().len(), 2, "{l}");
    assert!(
        text.contains("keep the speed") && text.contains("NEW ANSWER"),
        "{text}"
    );
    assert!(sp::unread(home).is_empty());
    // PUT THE OLD ONE BACK: the copy goes back to the capture's songs.
    sp::decide(home, &id, Answer::Undo, None).unwrap();
    assert!(!copy.join("sound/music.songs.json").exists());
    assert!(!copy.join("sound/banks.toml").exists());
    assert_eq!(sp::unread(home).len(), 1, "the agent hears the undo too");
    assert!(
        sp::decide(home, &id, Answer::Undo, None).is_err(),
        "one level"
    );
}

#[test]
fn a_sound_changed_since_is_not_replaced() {
    use crate::sound_proposal::{self as sp, Answer};
    let f = Fixture::new();
    let (v, err) = f.call(
        "sound_edit",
        json!({"copy": "copy", "ops": [{"op": "transpose", "song": "theme", "semitones": 2}], "why": "Higher."}),
    );
    assert!(!err, "{v}");
    let id = v["id"].as_str().unwrap();
    let (w, err) = f.call(
        "sound_edit",
        json!({"copy": "copy", "ops": [{"op": "transpose", "song": "theme", "semitones": 5}], "write": true, "confirmed_by_person": true}),
    );
    assert!(!err, "{w}");
    let e = sp::decide(&f.config.home, id, Answer::Accept, None).unwrap_err();
    assert!(e.contains("changed"), "{e}");
    assert_eq!(sp::read(&f.config.home, id).unwrap()["state"], "stale");
}

/// A role's sound for a large-art theme (issue #30): a new take proposed
/// with its look.txt line, the theme unchanged until ACCEPT, ACCEPT writing
/// both, PUT THE OLD ONE BACK taking the take and its line away; a change
/// to the role's own file kept as the old sound and refused as stale once
/// the file changed. Made-up tones, no game data.
#[test]
fn a_role_s_sound_is_proposed_and_only_the_person_s_answer_changes_the_theme() {
    use crate::sound_proposal::{self as sp, Answer, RoleSound};
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let root = home.join("work/themes/bright");
    let theme = root.join("theme");
    let tone = |period: usize| {
        let s: Vec<i16> = (0..400)
            .map(|i| {
                if (i / period).is_multiple_of(2) {
                    3000
                } else {
                    -3000
                }
            })
            .collect();
        gp_theme::tune::wav(&s, gp_present::sound::ROLE_RATE)
    };
    fs::create_dir_all(theme.join("sound/roles")).unwrap();
    let look = "gp-look 1\nid bright\ntitle Bright\nart art.txt\nfile sound/roles/hop.wav\n";
    fs::write(theme.join("look.txt"), look).unwrap();
    fs::write(theme.join("sound/roles/hop.wav"), tone(20)).unwrap();
    let two = tone(30);
    let with_two = format!("{look}file sound/takes/hop.2.wav\n");
    let v = sp::propose_role(
        &home,
        &RoleSound {
            theme: &root,
            theme_dir: &theme,
            title: "Bright",
            game: "made-up",
            role: "hop",
            take: 2,
            new: &two,
            new_look: &with_two,
            why: "A second hop, so it varies.",
            seconds: 0.01,
            old_seconds: 0.0,
            checked: json!({}),
        },
    )
    .unwrap();
    let id = v["id"].as_str().unwrap().to_owned();
    assert_eq!(v["file"], "sound/takes/hop.2.wav");
    assert_eq!(v["old_sha"], Value::Null);
    assert!(sp::wav(&home, &id, "old").is_none());
    assert_eq!(sp::wav(&home, &id, "new").unwrap(), two);
    assert!(!theme.join("sound/takes/hop.2.wav").exists());
    assert_eq!(fs::read_to_string(theme.join("look.txt")).unwrap(), look);
    assert_eq!(sp::for_next(&home)[0]["kind"], "role");
    assert!(
        sp::says(&v).starts_with("the sound \"hop\" (take 2) in Bright"),
        "{}",
        sp::says(&v)
    );
    // ACCEPT writes the take and the line; the undo takes both away.
    sp::decide(&home, &id, Answer::Accept, Some(" good ")).unwrap();
    assert_eq!(fs::read(theme.join("sound/takes/hop.2.wav")).unwrap(), two);
    assert_eq!(
        fs::read_to_string(theme.join("look.txt")).unwrap(),
        with_two
    );
    let u = sp::decide(&home, &id, Answer::Undo, None).unwrap();
    assert_eq!(u["state"], "undone");
    assert!(!theme.join("sound/takes/hop.2.wav").exists());
    assert_eq!(fs::read_to_string(theme.join("look.txt")).unwrap(), look);
    // A new first take: the one there is the old sound; changed after,
    // ACCEPT refuses it as stale and writes nothing.
    let one = tone(45);
    let w = sp::propose_role(
        &home,
        &RoleSound {
            theme: &root,
            theme_dir: &theme,
            title: "Bright",
            game: "made-up",
            role: "hop",
            take: 1,
            new: &one,
            new_look: look,
            why: "A softer hop.",
            seconds: 0.01,
            old_seconds: 0.01,
            checked: json!({}),
        },
    )
    .unwrap();
    let id2 = w["id"].as_str().unwrap().to_owned();
    assert_eq!(sp::wav(&home, &id2, "old").unwrap(), tone(20));
    fs::write(theme.join("sound/roles/hop.wav"), tone(25)).unwrap();
    let e = sp::decide(&home, &id2, Answer::Accept, None).unwrap_err();
    assert!(e.contains("changed in the theme"), "{e}");
    assert_eq!(
        fs::read(theme.join("sound/roles/hop.wav")).unwrap(),
        tone(25)
    );
    assert_eq!(sp::read(&home, &id2).unwrap()["state"], "stale");
    // The same file and line again is nothing to propose.
    assert!(
        sp::propose_role(
            &home,
            &RoleSound {
                theme: &root,
                theme_dir: &theme,
                title: "Bright",
                game: "made-up",
                role: "hop",
                take: 1,
                new: &tone(25),
                new_look: look,
                why: "Same.",
                seconds: 0.01,
                old_seconds: 0.01,
                checked: json!({}),
            },
        )
        .is_err()
    );
}
