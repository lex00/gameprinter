//! What sounds when (docs/sound.md, "Playback"): the model's sound
//! bindings compiled into cues, and a [`Director`] that reads the world
//! and the frame's events after each tick and says which song should play
//! and which effects start. Like the [`crate::Animator`] it reads the
//! world and never writes it, keeps to whole numbers, and every runtime
//! steps it the same way: the web preview drives gp-sound's player by its
//! word.

use gp_sim::World;
use gp_sim::systems::Event;

/// Header line of the cues text.
pub const HEADER: &str = "gp-sound-cues 1";
/// Header line of a cues text with sound roles (`role` lines).
pub const HEADER_ROLES: &str = "gp-sound-cues 2";

/// A sound role's cue (model feature `present.sound_roles`): the counter
/// holding the game's request and the test on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleCue {
    pub name: String,
    pub music: bool,
    /// Counter index in the spec.
    pub counter: usize,
    pub mask: i64,
    /// The masked value the cue wants; `None` for any but 0.
    pub equals: Option<i64>,
    pub held: bool,
    pub loops: bool,
}

impl RoleCue {
    /// Whether the world holds the role's request now.
    pub fn holds(&self, world: &World) -> bool {
        let v = world.counters.get(self.counter).copied().unwrap_or(0) & self.mask;
        match self.equals {
            Some(e) => v == e,
            None => v != 0,
        }
    }
}

/// The cues: songs by level and by rule state, effects by event. Songs and
/// effects are named by a key the runtime looks up (a file stem).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cues {
    /// (level index, song)
    pub level: Vec<(usize, String)>,
    /// (rules machine index, state index, song)
    pub state: Vec<(usize, usize, String)>,
    /// (event kind index, effect)
    pub sfx: Vec<(usize, String)>,
    /// Sound roles, in the model's order.
    pub roles: Vec<RoleCue>,
    /// Tests while any of which the game makes no sound (model feature
    /// `present.sound_silent_while`), as role cues with no name.
    pub silent: Vec<RoleCue>,
}

impl Cues {
    pub fn is_empty(&self) -> bool {
        self.level.is_empty()
            && self.state.is_empty()
            && self.sfx.is_empty()
            && self.roles.is_empty()
    }

    /// Whether the game makes no sound now.
    pub fn silent(&self, world: &World) -> bool {
        self.silent.iter().any(|t| t.holds(world))
    }

    pub fn write(&self) -> String {
        let header = if self.roles.is_empty() && self.silent.is_empty() {
            HEADER
        } else {
            HEADER_ROLES
        };
        let mut s = format!("{header}\n");
        for (l, k) in &self.level {
            s.push_str(&format!("music level {l} {k}\n"));
        }
        for (m, st, k) in &self.state {
            s.push_str(&format!("music state {m} {st} {k}\n"));
        }
        for (e, k) in &self.sfx {
            s.push_str(&format!("sfx {e} {k}\n"));
        }
        for r in &self.roles {
            s.push_str(&format!(
                "role {} {} {} {} {} {} {}\n",
                r.name,
                if r.music { "music" } else { "sfx" },
                r.counter,
                r.mask,
                r.equals.map_or("-".to_owned(), |e| e.to_string()),
                u8::from(r.held),
                u8::from(r.loops)
            ));
        }
        for t in &self.silent {
            s.push_str(&format!(
                "silent {} {} {}\n",
                t.counter,
                t.mask,
                t.equals.map_or("-".to_owned(), |e| e.to_string())
            ));
        }
        s
    }

    pub fn read(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        let first = lines.next();
        if first != Some(HEADER) && first != Some(HEADER_ROLES) {
            return Err("not a sound cues file".into());
        }
        let mut c = Cues::default();
        for (i, line) in lines.enumerate() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let n = |s: &str| {
                s.parse::<usize>()
                    .map_err(|_| format!("line {}: `{s}` isn't a number", i + 2))
            };
            match f.as_slice() {
                [] => {}
                ["music", "level", l, k] => c.level.push((n(l)?, (*k).to_owned())),
                ["music", "state", m, s, k] => c.state.push((n(m)?, n(s)?, (*k).to_owned())),
                ["sfx", e, k] => c.sfx.push((n(e)?, (*k).to_owned())),
                ["silent", counter, mask, equals] => {
                    let int = |s: &str| {
                        s.parse::<i64>()
                            .map_err(|_| format!("line {}: `{s}` isn't a number", i + 2))
                    };
                    c.silent.push(RoleCue {
                        name: String::new(),
                        music: false,
                        counter: n(counter)?,
                        mask: int(mask)?,
                        equals: if *equals == "-" {
                            None
                        } else {
                            Some(int(equals)?)
                        },
                        held: false,
                        loops: false,
                    })
                }
                ["role", name, kind, counter, mask, equals, held, loops] => {
                    let int = |s: &str| {
                        s.parse::<i64>()
                            .map_err(|_| format!("line {}: `{s}` isn't a number", i + 2))
                    };
                    c.roles.push(RoleCue {
                        name: (*name).to_owned(),
                        music: *kind == "music",
                        counter: n(counter)?,
                        mask: int(mask)?,
                        equals: if *equals == "-" {
                            None
                        } else {
                            Some(int(equals)?)
                        },
                        held: *held == "1",
                        loops: *loops == "1",
                    })
                }
                _ => return Err(format!("line {}: `{line}`", i + 2)),
            }
        }
        Ok(c)
    }
}

/// What changes this frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cue {
    /// The song that should play now, when it differs from the last
    /// frame's (`Some(None)`: stop the music).
    pub music: Option<Option<String>>,
    /// Effects that start this frame, in event order.
    pub sfx: Vec<String>,
    /// Sound roles that start this frame, in the model's order. A music
    /// role replaces the music role playing.
    pub starts: Vec<String>,
    /// Held sound roles whose request went this frame: stop them.
    pub stops: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Director {
    playing: Option<String>,
    /// Whether each role's test held last frame.
    held: Vec<bool>,
}

impl Director {
    pub fn new() -> Self {
        Self::default()
    }

    /// The song for the world as it is: a rule state's music over the
    /// level's.
    pub fn song_for(cues: &Cues, world: &World) -> Option<String> {
        cues.state
            .iter()
            .find(|(m, s, _)| world.machines.get(*m).is_some_and(|ms| ms.state == *s))
            .map(|(_, _, k)| k.clone())
            .or_else(|| {
                cues.level
                    .iter()
                    .find(|(l, _)| *l == world.level)
                    .map(|(_, k)| k.clone())
            })
    }

    /// Steps after a tick, with the tick's events.
    pub fn step(&mut self, cues: &Cues, world: &World, events: &[Event]) -> Cue {
        let want = Self::song_for(cues, world);
        let music = (want != self.playing).then(|| want.clone());
        self.playing = want;
        let sfx = events
            .iter()
            .filter_map(|e| {
                cues.sfx
                    .iter()
                    .find(|(k, _)| *k == e.kind.0)
                    .map(|(_, s)| s.clone())
            })
            .collect();
        let (starts, stops) = self.roles(cues, world);
        Cue {
            music,
            sfx,
            starts,
            stops,
        }
    }

    /// The sound roles' cues alone: which start and which stop.
    pub fn roles(&mut self, cues: &Cues, world: &World) -> (Vec<String>, Vec<String>) {
        self.held.resize(cues.roles.len(), false);
        let mut starts = Vec::new();
        let mut stops = Vec::new();
        for (r, was) in cues.roles.iter().zip(self.held.iter_mut()) {
            let now = r.holds(world);
            if r.held {
                if now && !*was {
                    starts.push(r.name.clone());
                }
                if !now && *was {
                    stops.push(r.name.clone());
                }
            } else if now {
                starts.push(r.name.clone());
            }
            *was = now;
        }
        (starts, stops)
    }
}

/// The rate a sound role's samples play at: the synthesizer's, so a
/// runtime streams them as it streams a driver's writes.
pub const ROLE_RATE: u32 = 44_100;

/// Samples a frame at [`ROLE_RATE`] and 60 frames a second.
pub const ROLE_FRAME: usize = 735;

/// A sound role playing: its role's index, which of the role's takes, and
/// where in its samples it is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Voice {
    role: usize,
    take: usize,
    at: usize,
}

/// The most starts [`RolePlayer::log`] keeps; later ones aren't logged.
pub const LOG_MAX: usize = 100_000;

/// A theme's sound per role, played on the roles' cues (theme format
/// 0.1.3, `[[sound]]`; docs/sound.md, "A theme's sound per role"). Every
/// runtime steps it after each tick and plays the frame's samples it
/// gives, so the Bevy build and the web preview play the same samples at
/// the same frames. While it holds any sound, the model's own sound (its
/// bound songs or a driver's APU writes) is silent: a role the theme gives
/// no file stays silent, so with a file for every role nothing reads the
/// game's sound data.
///
/// The cues: a role starts on the frame its test holds (a held role on
/// the first such frame only, and it stops when the test fails). A music
/// role replaces the music role playing; an effect starts again from its
/// beginning when it starts while it plays; a looping role (`loops`)
/// repeats while it plays. Samples are added and clipped to 16 bits.
///
/// A role may have several takes (a large-art look's
/// `sound/roles/<role>.wav` and `sound/takes/<role>.<n>.wav`, issue #30):
/// each start plays the next one, in the order of their numbers, and after
/// the last the first again. The turn is counted by this player from its
/// first frame (and afresh after [`RolePlayer::clear`]), never from a clock
/// or a random draw of its own, so a run played again from power-on plays
/// the same takes at the same frames.
#[derive(Debug, Clone, Default)]
pub struct RolePlayer {
    /// Each cue role's takes, by the cues' order: (take number, samples at
    /// [`ROLE_RATE`]), in number order; empty for a role with no file.
    sounds: Vec<Vec<(u32, Vec<i16>)>>,
    /// How many times each role has started.
    started: Vec<u64>,
    director: Director,
    /// The cues the last frame read, which start and stop on this one.
    pending: (Vec<String>, Vec<String>),
    music: Option<Voice>,
    sfx: Vec<Voice>,
    /// Every role started so far: (frame, role name, take number), for a
    /// report; at most [`LOG_MAX`].
    pub log: Vec<(u64, String, u32)>,
    /// Whether to keep [`RolePlayer::log`].
    pub keep_log: bool,
}

impl RolePlayer {
    /// A player for `cues`' roles, with no sounds yet.
    pub fn new(cues: &Cues) -> Self {
        Self {
            sounds: vec![Vec::new(); cues.roles.len()],
            started: vec![0; cues.roles.len()],
            ..Self::default()
        }
    }

    fn index(&mut self, cues: &Cues, role: &str) -> Option<usize> {
        let i = cues.roles.iter().position(|r| r.name == role)?;
        self.sounds.resize(cues.roles.len(), Vec::new());
        self.started.resize(cues.roles.len(), 0);
        Some(i)
    }

    /// Sets a role's samples, at [`ROLE_RATE`], as its one take; false
    /// when `cues` has no such role.
    pub fn set(&mut self, cues: &Cues, role: &str, samples: Vec<i16>) -> bool {
        let Some(i) = self.index(cues, role) else {
            return false;
        };
        self.sounds[i] = if samples.is_empty() {
            Vec::new()
        } else {
            vec![(1, samples)]
        };
        true
    }

    /// Sets take `take` (1 is the role's own file) of a role, keeping its
    /// other takes; empty samples take that take away. False when `cues`
    /// has no such role or `take` is 0.
    pub fn set_take(&mut self, cues: &Cues, role: &str, take: u32, samples: Vec<i16>) -> bool {
        if take == 0 {
            return false;
        }
        let Some(i) = self.index(cues, role) else {
            return false;
        };
        let t = &mut self.sounds[i];
        t.retain(|(n, _)| *n != take);
        if !samples.is_empty() {
            let at = t.partition_point(|(n, _)| *n < take);
            t.insert(at, (take, samples));
        }
        true
    }

    /// How many takes a role has.
    pub fn takes(&self, cues: &Cues, role: &str) -> usize {
        cues.roles
            .iter()
            .position(|r| r.name == role)
            .and_then(|i| self.sounds.get(i))
            .map_or(0, Vec::len)
    }

    /// Takes every role's sound away and stops what plays: the model's own
    /// sound plays again. The cues start afresh, so a held role (a stage's
    /// loop) starts on the next frame its request holds, as when a look
    /// changes mid-game, and each role's turn of takes starts again at its
    /// first.
    pub fn clear(&mut self) {
        for s in &mut self.sounds {
            s.clear();
        }
        self.started.fill(0);
        self.music = None;
        self.sfx.clear();
        self.director = Director::new();
        self.pending = (Vec::new(), Vec::new());
    }

    /// Whether any role has a sound: then the model's own sound is silent.
    pub fn is_on(&self) -> bool {
        self.sounds.iter().any(|s| !s.is_empty())
    }

    /// The roles that have a sound.
    pub fn roles<'a>(&'a self, cues: &'a Cues) -> impl Iterator<Item = &'a str> + 'a {
        cues.roles
            .iter()
            .zip(&self.sounds)
            .filter(|(_, s)| !s.is_empty())
            .map(|(r, _)| r.name.as_str())
    }

    /// The log as text, a start a line: `FRAME ROLE TAKE`.
    pub fn log_text(&self) -> String {
        let mut s = String::new();
        for (f, r, t) in &self.log {
            s.push_str(&format!("{f} {r} {t}\n"));
        }
        s
    }

    /// Steps after a tick: reads the roles' cues from the world and gives
    /// the frame's `n` samples.
    pub fn frame(&mut self, cues: &Cues, world: &World, n: usize) -> Vec<i16> {
        // A cue read after this tick sounds from the next frame, as a
        // game's driver takes the requests the last frame left (Donkey
        // Kong's at its NMI, $FA48): the driver's first note of a request
        // comes one frame after the request.
        let now = self.director.roles(cues, world);
        let (starts, stops) = std::mem::replace(&mut self.pending, now);
        // The game makes no sound now (an attract demo): nothing starts,
        // and what played stops.
        if cues.silent(world) {
            self.music = None;
            self.sfx.clear();
            self.pending = (Vec::new(), Vec::new());
            return vec![0; n];
        }
        self.sounds.resize(cues.roles.len(), Vec::new());
        self.started.resize(cues.roles.len(), 0);
        let index = |name: &str| cues.roles.iter().position(|r| r.name == name);
        for name in &stops {
            if let Some(i) = index(name) {
                if self.music.as_ref().is_some_and(|v| v.role == i) {
                    self.music = None;
                }
                self.sfx.retain(|v| v.role != i);
            }
        }
        for name in &starts {
            let Some(i) = index(name) else { continue };
            // The next take in turn; a role with none plays nothing, and
            // its turn doesn't move.
            let takes = self.sounds[i].len();
            let take = if takes == 0 {
                0
            } else {
                let k = (self.started[i] % takes as u64) as usize;
                self.started[i] += 1;
                k
            };
            if self.keep_log && self.log.len() < LOG_MAX {
                let number = self.sounds[i].get(take).map_or(0, |t| t.0);
                self.log.push((world.frame, name.clone(), number));
            }
            let v = Voice {
                role: i,
                take,
                at: 0,
            };
            if cues.roles[i].music {
                self.music = Some(v);
            } else {
                self.sfx.retain(|x| x.role != i);
                self.sfx.push(v);
            }
        }
        let mut mix = vec![0i32; n];
        let sounds = &self.sounds;
        let mut play = |v: &mut Voice| -> bool {
            let s: &[i16] = sounds[v.role].get(v.take).map_or(&[], |t| &t.1);
            let loops = cues.roles[v.role].loops;
            for m in &mut mix {
                if v.at >= s.len() {
                    if loops && !s.is_empty() {
                        v.at = 0;
                    } else {
                        return false;
                    }
                }
                *m += i32::from(s[v.at]);
                v.at += 1;
            }
            v.at < s.len() || (loops && !s.is_empty())
        };
        if let Some(v) = &mut self.music
            && !play(v)
        {
            self.music = None;
        }
        self.sfx.retain_mut(|v| play(v));
        mix.into_iter()
            .map(|v| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16)
            .collect()
    }
}

/// A 16-bit PCM WAV file's samples, mixed to mono and taken to
/// [`ROLE_RATE`]: the files the generator and `gp-web-prep` write for a
/// theme's sound roles (`sound/roles/<role>.wav`).
pub fn read_wav(bytes: &[u8]) -> Result<Vec<i16>, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF WAVE file".into());
    }
    let mut at = 12;
    let (mut channels, mut rate, mut bits) = (0u16, 0u32, 0u16);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        }
        if id == b"data" {
            if bits != 16 || channels == 0 || rate == 0 {
                return Err(format!(
                    "a {bits}-bit WAV with {channels} channels; give 16-bit PCM"
                ));
            }
            let c = usize::from(channels);
            let mono: Vec<i16> = body
                .chunks_exact(2 * c)
                .map(|f| {
                    let sum: i32 = f
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|b| i32::from(i16::from_le_bytes(*b)))
                        .sum();
                    (sum / c as i32) as i16
                })
                .collect();
            if rate == ROLE_RATE || mono.is_empty() {
                return Ok(mono);
            }
            // Nearest sample, in whole numbers.
            let n = (mono.len() as u64 * u64::from(ROLE_RATE) / u64::from(rate)) as usize;
            return Ok((0..n)
                .map(|i| mono[((i as u64 * u64::from(rate)) / u64::from(ROLE_RATE)) as usize])
                .collect());
        }
        at += 8 + len + (len & 1);
    }
    Err("the WAV file has no data".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cues_round_trip_and_the_director_follows_level_state_and_events() {
        let c = Cues {
            level: vec![(0, "theme".into())],
            state: vec![(0, 1, "death".into())],
            sfx: vec![(3, "stomp".into())],
            roles: Vec::new(),
            silent: Vec::new(),
        };
        assert_eq!(Cues::read(&c.write()).unwrap(), c);
        assert!(Cues::read("nope").is_err());
        let mut w = World::default();
        w.machines
            .push(gp_sim::world::MachineState { state: 0, time: 0 });
        let mut d = Director::new();
        let ev = |k: usize| Event {
            frame: 1,
            kind: gp_sim::spec::EventKind(k),
            entity: None,
        };
        let first = d.step(&c, &w, &[ev(3), ev(2)]);
        assert_eq!(first.music, Some(Some("theme".into())));
        assert_eq!(first.sfx, vec!["stomp".to_string()]);
        assert_eq!(d.step(&c, &w, &[]).music, None);
        w.machines[0].state = 1;
        assert_eq!(d.step(&c, &w, &[]).music, Some(Some("death".into())));
    }

    #[test]
    fn roles_round_trip_and_start_on_their_counters() {
        let c = Cues {
            roles: vec![
                RoleCue {
                    name: "jump".into(),
                    music: false,
                    counter: 0,
                    mask: 4,
                    equals: None,
                    held: false,
                    loops: false,
                },
                RoleCue {
                    name: "stage".into(),
                    music: true,
                    counter: 1,
                    mask: 255,
                    equals: Some(2),
                    held: true,
                    loops: true,
                },
            ],
            ..Cues::default()
        };
        let text = c.write();
        assert!(text.starts_with(HEADER_ROLES));
        assert_eq!(Cues::read(&text).unwrap(), c);
        let mut w = World {
            counters: vec![4, 2],
            ..World::default()
        };
        let mut d = Director::new();
        let a = d.step(&c, &w, &[]);
        assert_eq!(a.starts, vec!["jump".to_string(), "stage".to_string()]);
        // A cleared request starts again each frame it is set; a held one
        // only once, and stops when it goes.
        let b = d.step(&c, &w, &[]);
        assert_eq!(b.starts, vec!["jump".to_string()]);
        w.counters = vec![8, 0x40];
        let e = d.step(&c, &w, &[]);
        assert!(e.starts.is_empty());
        assert_eq!(e.stops, vec!["stage".to_string()]);
    }

    #[test]
    fn a_role_player_plays_each_role_on_its_cue_and_mixes_them() {
        let c = Cues {
            roles: vec![
                RoleCue {
                    name: "jump".into(),
                    music: false,
                    counter: 0,
                    mask: 1,
                    equals: None,
                    held: false,
                    loops: false,
                },
                RoleCue {
                    name: "stage".into(),
                    music: true,
                    counter: 1,
                    mask: -1,
                    equals: Some(2),
                    held: true,
                    loops: true,
                },
            ],
            ..Cues::default()
        };
        let mut p = RolePlayer::new(&c);
        p.keep_log = true;
        assert!(!p.is_on());
        assert!(p.set(&c, "jump", vec![100; 5]));
        assert!(p.set(&c, "stage", vec![1, 2, 3]));
        assert!(!p.set(&c, "nope", vec![1]));
        assert!(p.is_on());
        let mut w = World {
            counters: vec![0, 2],
            frame: 1,
            ..World::default()
        };
        // A cue sounds from the next frame, as a driver takes requests.
        assert_eq!(p.frame(&c, &w, 4), vec![0, 0, 0, 0]);
        w.counters = vec![1, 2];
        w.frame = 2;
        // The stage loops; the jump is asked for.
        assert_eq!(p.frame(&c, &w, 4), vec![1, 2, 3, 1]);
        w.counters = vec![0, 0];
        w.frame = 3;
        assert_eq!(p.frame(&c, &w, 3), vec![102, 103, 101]);
        w.frame = 4;
        // The stage stops a frame after its request goes; the jump plays on.
        assert_eq!(p.frame(&c, &w, 3), vec![100, 100, 0]);
        assert_eq!(
            p.log,
            vec![(2, "stage".to_string(), 1), (3, "jump".to_string(), 1)]
        );
        // While a silent test holds, nothing plays.
        let mut quiet = c.clone();
        quiet.silent.push(RoleCue {
            name: String::new(),
            music: false,
            counter: 0,
            mask: -1,
            equals: Some(9),
            held: false,
            loops: false,
        });
        assert_eq!(Cues::read(&quiet.write()).unwrap(), quiet);
        w.counters = vec![9, 2];
        assert_eq!(p.frame(&quiet, &w, 2), vec![0, 0]);
        p.clear();
        assert!(!p.is_on());
        let wav = {
            let mut b = Vec::new();
            b.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
            b.extend_from_slice(&16u32.to_le_bytes());
            b.extend_from_slice(&[1, 0, 1, 0]);
            b.extend_from_slice(&22_050u32.to_le_bytes());
            b.extend_from_slice(&44_100u32.to_le_bytes());
            b.extend_from_slice(&[2, 0, 16, 0]);
            b.extend_from_slice(b"data");
            b.extend_from_slice(&4u32.to_le_bytes());
            b.extend_from_slice(&7i16.to_le_bytes());
            b.extend_from_slice(&(-7i16).to_le_bytes());
            b
        };
        assert_eq!(read_wav(&wav).unwrap(), vec![7, 7, -7, -7]);
    }

    /// A role with three takes (issue #30): each start plays the next, in
    /// the order of their numbers whatever order they were set in, each
    /// take's own samples; a second player over the same frames plays the
    /// same takes at the same frames; `set` and `clear` leave one take and
    /// start the turn again.
    #[test]
    fn a_role_with_takes_plays_each_in_turn_and_the_same_again() {
        let c = Cues {
            roles: vec![RoleCue {
                name: "hop".into(),
                music: false,
                counter: 0,
                mask: -1,
                equals: Some(1),
                held: false,
                loops: false,
            }],
            ..Cues::default()
        };
        let run = || {
            let mut p = RolePlayer::new(&c);
            p.keep_log = true;
            // Set out of order: the turn goes by number.
            assert!(p.set_take(&c, "hop", 3, vec![30; 2]));
            assert!(p.set(&c, "hop", vec![10; 2]));
            assert!(p.takes(&c, "hop") == 1);
            assert!(p.set_take(&c, "hop", 3, vec![30; 2]));
            assert!(p.set_take(&c, "hop", 2, vec![20; 2]));
            assert!(!p.set_take(&c, "hop", 0, vec![1]));
            assert!(!p.set_take(&c, "nope", 2, vec![1]));
            assert_eq!(p.takes(&c, "hop"), 3);
            let mut heard = Vec::new();
            for f in 1..=40u64 {
                let w = World {
                    counters: vec![i64::from(f % 8 == 0)],
                    frame: f,
                    ..World::default()
                };
                let s = p.frame(&c, &w, 2);
                if s[0] != 0 {
                    heard.push((f, s[0]));
                }
            }
            (p, heard)
        };
        let (mut p, heard) = run();
        // Asked on frames 8, 16, 24, 32 and 40; each sounds from the next.
        assert_eq!(heard, vec![(9, 10), (17, 20), (25, 30), (33, 10)]);
        assert_eq!(
            p.log.iter().map(|x| (x.0, x.2)).collect::<Vec<_>>(),
            vec![(9, 1), (17, 2), (25, 3), (33, 1)]
        );
        assert_eq!(p.log_text().lines().next(), Some("9 hop 1"));
        let (again, heard_again) = run();
        assert_eq!(heard_again, heard);
        assert_eq!(again.log, p.log);
        // An empty take takes it away; clear starts the turn again.
        assert!(p.set_take(&c, "hop", 2, Vec::new()));
        assert_eq!(p.takes(&c, "hop"), 2);
        p.clear();
        assert!(!p.is_on());
        assert!(p.set(&c, "hop", vec![5; 2]));
        assert_eq!(p.takes(&c, "hop"), 1);
    }
}
