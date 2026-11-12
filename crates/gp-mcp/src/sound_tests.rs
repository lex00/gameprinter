//! The sound tools on the made-up game (crate::testgame) with a made-up
//! song: describe, the readiness gate, edits with clips, history and undo,
//! the live run's input, and the in-context preview. No game data.

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
        ], "prompt": "a lower lead"}),
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
    let (u, err) = f.call("sound_edit", json!({"copy": "copy", "undo": true}));
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
