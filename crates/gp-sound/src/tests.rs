//! Tests on made-up register logs and made-up songs; no game's data.

use crate::apu::{Channel, Region, period_of};
use crate::capture::{self, Options, Req};
use crate::log::{ApuLog, Write, cycle_in_frame};
use crate::song::{Kind, Request};

/// A made-up driver: a four-note phrase on pulse 1 with a decaying
/// volume, a bass note per phrase on the triangle and a beat on the
/// noise, the phrase played `loops` times, 16 frames a note.
pub fn made_up_log(loops: u32, jump_at: Option<u64>) -> ApuLog {
    let r = Region::Ntsc;
    let mut w: Vec<Write> = Vec::new();
    let mut put = |frame: u64, reg: u8, value: u8| {
        w.push(Write {
            frame,
            cycle: 100 + w.iter().filter(|x| x.frame == frame).count() as u32 * 12,
            reg,
            value,
        })
    };
    put(0, 0x15, 0x0F);
    let phrase = [60.0, 64.0, 67.0, 72.0];
    let mut f = 1;
    for _ in 0..loops {
        for (i, &m) in phrase.iter().enumerate() {
            let p = period_of(Channel::Pulse1, m, r);
            for k in 0..16u64 {
                let fr = f + i as u64 * 16 + k;
                let jumping = jump_at.is_some_and(|j| fr >= j && fr < j + 10);
                if !jumping {
                    let vol = 12u8.saturating_sub(k as u8 / 2);
                    put(fr, 0x00, 0x80 | 0x30 | vol);
                    put(fr, 0x02, (p & 0xFF) as u8);
                    if k == 0 {
                        put(fr, 0x03, (p >> 8) as u8 | 0x08);
                    }
                }
            }
            if i == 0 {
                let t = period_of(Channel::Triangle, 36.0, r);
                put(f, 0x08, 0xFF);
                put(f, 0x0A, (t & 0xFF) as u8);
                put(f, 0x0B, (t >> 8) as u8 | 0x08);
            }
            if i == 2 {
                put(f + 32, 0x08, 0x80);
            }
            put(f + i as u64 * 16, 0x0C, 0x30 | 8);
            put(f + i as u64 * 16, 0x0E, 0x03);
            put(f + i as u64 * 16, 0x0F, 0x08);
            put(f + i as u64 * 16 + 2, 0x0C, 0x30);
        }
        f += 64;
    }
    if let Some(j) = jump_at {
        // A rising effect on pulse 1 (written over the music's frames).
        for k in 0..10u64 {
            let p = 300 - k as u16 * 10;
            put(j + k, 0x00, 0x40 | 0x30 | 10);
            put(j + k, 0x02, (p & 0xFF) as u8);
            if k == 0 {
                put(j + k, 0x03, (p >> 8) as u8 | 0x08);
            }
        }
    }
    w.sort_by_key(|x| (x.frame, x.cycle));
    ApuLog {
        region: r,
        source: None,
        recording: Some("made-up".into()),
        first: 0,
        end: f,
        timed: true,
        writes: w,
        samples: Vec::new(),
    }
}

#[test]
fn log_text_round_trips() {
    let log = made_up_log(2, None);
    let text = log.to_text();
    assert_eq!(ApuLog::parse(&text).unwrap(), log);
    assert!(ApuLog::parse("gp-apu-log 9\n").is_err());
    assert!(ApuLog::parse("gp-apu-log 1\nframes 0 2\n1 0 18 00\n").is_err());
    assert!(ApuLog::parse("gp-apu-log 1\nframes 0 2\n5 0 00 00\n").is_err());
}

#[test]
fn cycles_in_frame_start_at_vblank() {
    assert_eq!(cycle_in_frame(Region::Ntsc, 241, 1), 0);
    assert_eq!(cycle_in_frame(Region::Ntsc, 242, 1), 113);
    assert_eq!(cycle_in_frame(Region::Ntsc, 0, 1), (21 * 341) / 3);
}

#[test]
fn notes_and_instruments_from_a_log() {
    let log = made_up_log(1, None);
    let states = log.states();
    let raw = capture::notes(&states, None);
    let pulse: Vec<_> = raw
        .iter()
        .filter(|n| n.channel == Channel::Pulse1)
        .collect();
    assert_eq!(pulse.len(), 4);
    assert_eq!(pulse[0].volume[..4], [12, 12, 11, 11]);
    assert_eq!(
        raw.iter()
            .filter(|n| n.channel == Channel::Triangle)
            .count(),
        1
    );
    assert_eq!(
        raw.iter().filter(|n| n.channel == Channel::Noise).count(),
        4
    );
    let mut ins = Vec::new();
    let of = capture::instruments(&raw, &mut ins);
    // The four pulse notes share an instrument, as do the four beats.
    let p: Vec<u16> = raw
        .iter()
        .zip(&of)
        .filter(|(n, _)| n.channel == Channel::Pulse1)
        .map(|(_, &i)| i)
        .collect();
    assert!(p.iter().all(|&i| i == p[0]));
    assert_eq!(ins.len(), 3);
}

#[test]
fn capture_finds_tempo_loop_and_plays_back_exactly() {
    let log = made_up_log(4, None);
    let states = log.states();
    let set = capture::capture(
        Region::Ntsc,
        &states,
        &[],
        &Options {
            name: "made up".into(),
            ..Options::default()
        },
    );
    assert_eq!(set.songs.len(), 1);
    let s = &set.songs[0];
    assert_eq!(s.name, "made_up");
    assert_eq!(s.tempo.frames_per_row, 16);
    let lf = s.loop_frame.expect("a loop");
    assert_eq!(s.frames - lf, 64, "the phrase's length");
    // Patterns repeat: the pulse track's columns share one pattern.
    // The song starts at its first note, frame 1 of the log.
    assert_eq!(s.tempo.rows_per_pattern, 4);
    let full = crate::synth::render(
        log.region,
        &log.timeline(),
        log.region.frame_start(log.frames()),
        &[],
        crate::synth::UNITY,
    );
    let audio = crate::frames_of(log.region, &full, 1, 1 + s.frames);
    let a = crate::agreement(
        &set,
        &s.name,
        &states[1..1 + s.frames as usize],
        Some(audio),
        None,
    )
    .unwrap();
    assert_eq!(a.f1, 1.0, "{a:?}");
    assert_eq!(a.pitch_frames, 1.0);
    assert_eq!(a.volume_frames, 1.0);
    assert!(a.loudness.unwrap() > 0.95, "{a:?}");
    let roles: Vec<_> = set
        .instruments
        .iter()
        .map(|i| i.role.clone().unwrap())
        .collect();
    assert!(roles.contains(&"bass".to_string()) && roles.contains(&"drums".to_string()));
}

#[test]
fn an_effect_claims_its_channel_and_the_music_keeps_the_rest() {
    let log = made_up_log(4, Some(40));
    let states = log.states();
    let reqs = vec![Req {
        frame: 40,
        request: Request {
            via: "Square1SoundQueue".into(),
            id: 1,
            name: "Sfx_Jump".into(),
        },
        kind: Kind::Sfx,
    }];
    let set = capture::capture(Region::Ntsc, &states, &reqs, &Options::default());
    let fx = set
        .songs
        .iter()
        .find(|s| s.kind == Kind::Sfx)
        .expect("an effect");
    assert_eq!(fx.name, "sfx_jump");
    assert_eq!(fx.channels(), vec![Channel::Pulse1]);
    assert_eq!(fx.frames, 10);
    let music = set.songs.iter().find(|s| s.kind == Kind::Music).unwrap();
    assert!(
        music.loop_frame.is_some(),
        "the loop is found around the effect"
    );
}

#[test]
fn synth_is_deterministic_and_makes_sound() {
    let log = made_up_log(1, None);
    let end = log.region.frame_start(log.frames());
    let a = crate::synth::render(log.region, &log.timeline(), end, &[], crate::synth::UNITY);
    let b = crate::synth::render(log.region, &log.timeline(), end, &[], crate::synth::UNITY);
    assert_eq!(a, b);
    assert!(a.iter().any(|&s| s.unsigned_abs() > 1000));
    let wav = crate::synth::wav(&a, crate::synth::SAMPLE_RATE);
    assert_eq!(&wav[..4], b"RIFF");
    // Frame by frame gives the same samples as the whole.
    let mut s = crate::synth::Synth::new(log.region, crate::synth::SAMPLE_RATE, Vec::new());
    let mut c = Vec::new();
    for f in log.first..log.end {
        let w: Vec<(u32, u8, u8)> = log
            .writes
            .iter()
            .filter(|w| w.frame == f)
            .map(|w| (w.cycle, w.reg, w.value))
            .collect();
        c.extend(s.frame(&w));
    }
    assert_eq!(a, c);
}

#[test]
fn names_bind_to_events_and_states() {
    let events: Vec<String> = [
        "enemy.stomped",
        "item.collected",
        "player.died",
        "enemy.kicked",
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(
        capture::bind("Sfx_EnemyStomp", &events).as_deref(),
        Some("enemy.stomped")
    );
    assert_eq!(
        capture::bind("Sfx_CoinGrab", &events).as_deref(),
        Some("item.collected")
    );
    assert_eq!(capture::bind("Square1SoundQueue 0x01", &events), None);
    let states: Vec<String> = ["game.dying", "game.playing", "game.game_over"]
        .map(String::from)
        .to_vec();
    assert_eq!(
        capture::bind("death", &states).as_deref(),
        Some("game.dying")
    );
    assert_eq!(capture::words("Sfx_EnemyStomp"), vec!["enemy", "stomp"]);
}

/// A made-up 0.1 file, as an older gp wrote one: an NES pulse note.
const SONGS_01: &str = r#"{
  "format": "gp-songs", "version": "0.1.0", "region": "ntsc",
  "instruments": [{"name": "lead", "voice": "pulse", "volume": [15, 12], "duty": [2]}],
  "songs": [{"name": "tune", "kind": "music", "origin": "edited", "confidence": 1,
    "frames": 16, "tempo": {"frames_per_row": 4, "rows_per_pattern": 4},
    "tracks": [{"channel": "pulse1", "patterns": [{"notes": [{"at": 0, "len": 8, "pitch": 69, "instrument": 0}]}], "order": [0]}]}]
}"#;

/// A made-up 0.2 set for an arcade board: a tone, its chip's noise and
/// envelope.
fn ay_set() -> crate::song::SongSet {
    let text = r#"{
  "format": "gp-songs", "version": "0.2.0", "region": "ntsc", "clock": 2000000,
  "instruments": [
    {"name": "buzz", "voice": "tone", "volume": [16], "duty": [3]},
    {"name": "hiss", "voice": "ay-noise", "volume": [15]},
    {"name": "saw", "voice": "envelope", "volume": [15]}],
  "songs": [{"name": "tune", "kind": "music", "origin": "edited", "confidence": 1,
    "frames": 16, "tempo": {"frames_per_row": 4, "rows_per_pattern": 4},
    "tracks": [
      {"channel": "ay1.b", "patterns": [{"notes": [{"at": 0, "len": 8, "pitch": 69, "fine": 2, "instrument": 0}]}], "order": [0]},
      {"channel": "ay1.noise", "patterns": [{"notes": [{"at": 0, "len": 8, "pitch": 31, "instrument": 1}]}], "order": [0]},
      {"channel": "ay1.envelope", "patterns": [{"notes": [{"at": 0, "len": 8, "pitch": 8, "period": 40000, "instrument": 2}]}], "order": [0]}]}]
}"#;
    crate::song::SongSet::parse(text).unwrap()
}

#[test]
fn songs_0_1_still_read_and_write_as_0_1() {
    use crate::song::SongSet;
    let set = SongSet::parse(SONGS_01).unwrap();
    assert_eq!(set.version, "0.1.0");
    assert!(!set.has_ay());
    let back = SongSet::parse(&set.to_text()).unwrap();
    assert_eq!(back, set);
    assert!(set.to_text().contains("\"version\": \"0.1.0\""));
    // A new set with nothing of 0.2 says 0.1.0, so older readers take it.
    assert_eq!(SongSet::new(Region::Ntsc).version, "0.1.0");
    // Other versions: a 0.2 patch reads, 0.3 and 1.0 don't.
    for (v, ok) in [
        ("0.1.7", true),
        ("0.2.4", true),
        ("0.3.0", false),
        ("1.0.0", false),
        ("0.0.1", false),
    ] {
        let t = SONGS_01.replace("\"0.1.0\"", &format!("\"{v}\""));
        assert_eq!(SongSet::parse(&t).is_ok(), ok, "{v}");
    }
}

#[test]
fn songs_0_2_hold_ay_channels() {
    use crate::apu::{AyPart, Channel};
    use crate::song::{SongSet, Voice, ay_pitch};
    let set = ay_set();
    assert!(set.has_ay());
    let song = set.song("tune").unwrap();
    assert_eq!(
        song.channels(),
        vec![Channel::Ay1B, Channel::Ay1Noise, Channel::Ay1Envelope]
    );
    assert_eq!(Channel::Ay1B.ay(), Some((1, AyPart::Tone(1))));
    assert_eq!(Channel::Ay0Envelope.ay(), Some((0, AyPart::Envelope)));
    assert_eq!(Channel::Pulse2.ay(), None);
    assert_eq!(Voice::of(Channel::Ay0Noise), Voice::AyNoise);
    assert_eq!(Channel::parse_any("ay1.noise"), Some(Channel::Ay1Noise));
    assert_eq!(Channel::parse("ay1.noise"), None);
    // A440 on a 2 MHz AY: 2e6 / 16 / 440 = 284.09, so 284, then the fine
    // offset.
    let tone = &song.track(Channel::Ay1B).unwrap().patterns[0].notes[0];
    assert_eq!(set.period(Channel::Ay1B, tone), 286);
    assert!((ay_pitch(284, None).unwrap() - 69.0).abs() < 0.01);
    let noise = &song.track(Channel::Ay1Noise).unwrap().patterns[0].notes[0];
    assert_eq!(set.period(Channel::Ay1Noise, noise), 31);
    let env = &song.track(Channel::Ay1Envelope).unwrap().patterns[0].notes[0];
    assert_eq!(set.period(Channel::Ay1Envelope, env), 40000);
    // It writes and reads back the same, as 0.2.0.
    let text = set.to_text();
    assert!(text.contains("\"ay1.envelope\"") && text.contains("\"ay-noise\""));
    assert_eq!(SongSet::parse(&text).unwrap(), set);
    // A set read as 0.1 that gained an AY track writes 0.2.0.
    let mut old = SongSet::parse(SONGS_01).unwrap();
    old.instruments.extend(set.instruments.clone());
    let mut s = set.songs[0].clone();
    s.name = "arcade".into();
    for t in &mut s.tracks {
        for p in &mut t.patterns {
            for n in &mut p.notes {
                n.instrument += 1;
            }
        }
    }
    old.songs.push(s);
    let text = old.to_text();
    assert!(text.contains("\"version\": \"0.2.0\""));
    assert!(SongSet::parse(&text).is_ok());
    // A file that says 0.1 can't hold them.
    let t = set.to_text().replace("\"0.2.0\"", "\"0.1.0\"");
    assert!(SongSet::parse(&t).unwrap_err().contains("came in 0.2.0"));
    // An APU instrument on an AY channel is refused.
    let t = set
        .to_text()
        .replace("\"voice\": \"tone\"", "\"voice\": \"pulse\"");
    assert!(SongSet::parse(&t).is_err());
}

#[test]
fn the_apu_player_leaves_ay_tracks_alone() {
    let mut p = crate::player::Player::new(ay_set());
    p.play_music(Some("tune"));
    for _ in 0..20 {
        let w = p.frame();
        // Only the silencing writes of the APU's channels, never a note.
        assert!(w.iter().all(|&(_, r, v)| r != 0x02 || v == 0), "{w:?}");
    }
}

/// Prints a captured song's frames beside its log's (GP_SOUND_DEBUG=dir:song).
#[test]
#[ignore]
fn debug_song() {
    let Ok(spec) = std::env::var("GP_SOUND_DEBUG") else {
        return;
    };
    let (dir, song) = spec.split_once(':').unwrap();
    let log = ApuLog::parse(&std::fs::read_to_string(format!("{dir}/apu.log")).unwrap()).unwrap();
    let set =
        crate::song::SongSet::parse(&std::fs::read_to_string(format!("{dir}/songs.json")).unwrap())
            .unwrap();
    let s = set.song(song).unwrap();
    let a = s.captured.as_ref().unwrap().frames[0];
    let sub = log.slice(a, a + s.frames);
    let r = sub.states();
    let (tl, n) = crate::player::song_timeline(&set, song).unwrap();
    let c = crate::log::states_of(set.region, &tl, n, &set.samples);
    for f in 0..(n as usize).min(80) {
        let fmt = |x: &[crate::apu::ChannelState; 5]| {
            x.iter()
                .map(|s| {
                    format!(
                        "{:4}/{:2}/{}{}",
                        s.period,
                        s.volume,
                        s.duty,
                        if s.trigger { "*" } else { " " }
                    )
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        eprintln!("{f:3} ref {}\n    got {}", fmt(&r[f]), fmt(&c[f]));
    }
}
