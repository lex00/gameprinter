//! Deterministic edits on a song set and its sound bank mapping, for an
//! agent asked "make the bass a real bass guitar" or "play 1-1 a bit
//! slower" (docs/research/agent-sprite-editor.md has the pattern; gp-mcp's
//! `sound_edit` runs these). Each op checks what it would make against
//! the NES's limits and refuses rather than clamp.

use serde::{Deserialize, Serialize};

use crate::apu::Channel;
use crate::banks::{Mapping, ROLES, Rule, Target};
use crate::song::{Kind, Placed, SongSet, Track, Voice};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Plays an instrument, or every instrument of a role, through a
    /// target (the APU, a GM program or a bank's patch), in one song or
    /// all.
    Remap {
        #[serde(default)]
        instrument: Option<String>,
        #[serde(default)]
        role: Option<String>,
        #[serde(default)]
        song: Option<String>,
        target: Target,
        #[serde(default)]
        gain: Option<f64>,
    },
    /// Gives an instrument a role.
    Role {
        instrument: String,
        role: String,
    },
    /// Moves a song's pitched notes (pulse, triangle) by semitones, or one
    /// channel's.
    Transpose {
        song: String,
        semitones: i8,
        #[serde(default)]
        channel: Option<String>,
    },
    /// Plays a song at a new speed: frames per row from the old to the new,
    /// every note's start and length scaled.
    Tempo {
        song: String,
        frames_per_row: u32,
    },
    /// Mutes channels, or solos them (the rest muted); an empty list
    /// clears it.
    Mute {
        channels: Vec<String>,
    },
    Solo {
        channels: Vec<String>,
    },
    /// A channel's or a role's loudness, 1.0 as captured.
    Gain {
        #[serde(default)]
        channel: Option<String>,
        #[serde(default)]
        role: Option<String>,
        value: f64,
    },
    /// Makes a song the music of a level (or a rule state), in place of
    /// the one there.
    Swap {
        song: String,
        #[serde(default)]
        level: Option<String>,
        #[serde(default)]
        state: Option<String>,
    },
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Remap { .. } => "remap",
            Self::Role { .. } => "role",
            Self::Transpose { .. } => "transpose",
            Self::Tempo { .. } => "tempo",
            Self::Mute { .. } => "mute",
            Self::Solo { .. } => "solo",
            Self::Gain { .. } => "gain",
            Self::Swap { .. } => "swap",
        }
    }
}

fn channels(v: &[String]) -> Result<Vec<String>, String> {
    for c in v {
        if Channel::parse(c).is_none() {
            return Err(format!(
                "`{c}` isn't a channel (pulse1, pulse2, triangle, noise, dmc)"
            ));
        }
    }
    Ok(v.to_vec())
}

/// Runs ops in order on copies; returns what changed, or the first
/// refusal (nothing changes then).
pub fn apply(
    set: &SongSet,
    map: &Mapping,
    ops: &[Op],
) -> Result<(SongSet, Mapping, Vec<String>), String> {
    let mut set = set.clone();
    let mut map = map.clone();
    let mut said = Vec::new();
    for op in ops {
        match op {
            Op::Remap {
                instrument,
                role,
                song,
                target,
                gain,
            } => {
                if instrument.is_none() && role.is_none() {
                    return Err("remap names an instrument or a role".into());
                }
                if let Some(i) = instrument
                    && !set.instruments.iter().any(|x| &x.name == i)
                {
                    return Err(format!("there's no instrument `{i}`"));
                }
                if let Some(s) = song
                    && set.song(s).is_none()
                {
                    return Err(format!("there's no song `{s}`"));
                }
                map.rules.retain(|r| {
                    !(r.instrument == *instrument && r.role == *role && r.song == *song)
                });
                map.rules.insert(
                    0,
                    Rule {
                        instrument: instrument.clone(),
                        role: role.clone(),
                        song: song.clone(),
                        target: target.clone(),
                        gain: *gain,
                        transpose: None,
                    },
                );
                map.check()?;
                said.push(format!(
                    "{} plays through {}",
                    instrument
                        .clone()
                        .or_else(|| role.as_ref().map(|r| format!("every {r} instrument")))
                        .unwrap_or_default(),
                    match target {
                        Target::Apu => "the APU".to_owned(),
                        Target::Gm { program } => format!("GM program {program}"),
                        Target::Bank {
                            bank,
                            preset,
                            program,
                            ..
                        } => format!(
                            "{bank} {}",
                            preset
                                .clone()
                                .or(program.map(|p| format!("program {p}")))
                                .unwrap_or_default()
                        ),
                    }
                ));
            }
            Op::Role { instrument, role } => {
                if !ROLES.contains(&role.as_str()) {
                    return Err(format!("role `{role}` isn't one of {}", ROLES.join(", ")));
                }
                let ins = set
                    .instruments
                    .iter_mut()
                    .find(|x| &x.name == instrument)
                    .ok_or_else(|| format!("there's no instrument `{instrument}`"))?;
                ins.role = Some(role.clone());
                said.push(format!("{instrument} is {role}"));
            }
            Op::Transpose {
                song,
                semitones,
                channel,
            } => {
                let only = channel
                    .as_deref()
                    .map(|c| Channel::parse(c).ok_or_else(|| format!("`{c}` isn't a channel")));
                let only = only.transpose()?;
                let region = set.region;
                let s = set
                    .song_mut(song)
                    .ok_or_else(|| format!("there's no song `{song}`"))?;
                let tempo = s.tempo;
                let frames = s.frames;
                for t in &mut s.tracks {
                    if only.is_some_and(|c| c != t.channel)
                        || !matches!(Voice::of(t.channel), Voice::Pulse | Voice::Triangle)
                    {
                        continue;
                    }
                    let mut placed = t.placed(tempo);
                    for p in &mut placed {
                        let n = i16::from(p.note.pitch) + i16::from(*semitones);
                        if !(0..=127).contains(&n) {
                            return Err(format!("{song}: a note would leave the MIDI range"));
                        }
                        p.note.pitch = n as u8;
                        let period = p.note.period(t.channel, region);
                        let low = if t.channel == Channel::Triangle { 2 } else { 8 };
                        if period < low || period >= 0x7FF {
                            return Err(format!(
                                "{song}: moving {} by {semitones} takes a note out of its range",
                                t.channel.name()
                            ));
                        }
                    }
                    *t = Track::from_placed(t.channel, &placed, tempo, frames);
                }
                said.push(format!("{song} moved {semitones} semitones"));
            }
            Op::Tempo {
                song,
                frames_per_row,
            } => {
                if *frames_per_row == 0 || *frames_per_row > 64 {
                    return Err("frames per row is 1 to 64".into());
                }
                let s = set
                    .song_mut(song)
                    .ok_or_else(|| format!("there's no song `{song}`"))?;
                let old = u64::from(s.tempo.frames_per_row.max(1));
                let new = u64::from(*frames_per_row);
                let scale = |x: u64| (x * new + old / 2) / old;
                let tempo = crate::song::Tempo {
                    frames_per_row: *frames_per_row,
                    rows_per_pattern: s.tempo.rows_per_pattern,
                };
                let frames = scale(s.frames).max(1);
                let ot = s.tempo;
                for t in &mut s.tracks {
                    let placed: Vec<Placed> = t
                        .placed(ot)
                        .into_iter()
                        .map(|mut p| {
                            let end = scale(p.start + u64::from(p.note.len));
                            p.start = scale(p.start);
                            p.note.len = (end - p.start).max(1) as u32;
                            p
                        })
                        .collect();
                    *t = Track::from_placed(t.channel, &placed, tempo, frames);
                }
                s.frames = frames;
                s.loop_frame = s.loop_frame.map(scale);
                s.tempo = tempo;
                said.push(format!(
                    "{song} at {frames_per_row} frames a row (was {old})"
                ));
            }
            Op::Mute { channels: c } => {
                map.mix.mute = channels(c)?;
                said.push(format!("muted: {}", map.mix.mute.join(", ")));
            }
            Op::Solo { channels: c } => {
                map.mix.solo = channels(c)?;
                said.push(format!("solo: {}", map.mix.solo.join(", ")));
            }
            Op::Gain {
                channel,
                role,
                value,
            } => {
                if !(0.0..=4.0).contains(value) {
                    return Err("a gain is 0 to 4".into());
                }
                match (channel, role) {
                    (Some(c), None) => {
                        channels(std::slice::from_ref(c))?;
                        map.mix.gain.insert(c.clone(), *value);
                    }
                    (None, Some(r)) if ROLES.contains(&r.as_str()) => {
                        map.mix.role_gain.insert(r.clone(), *value);
                    }
                    _ => return Err("gain takes a channel or a role".into()),
                }
                said.push(format!(
                    "{} at {value}",
                    channel.clone().or(role.clone()).unwrap_or_default()
                ));
            }
            Op::Swap { song, level, state } => {
                if level.is_none() == state.is_none() {
                    return Err("swap takes a level or a state".into());
                }
                let k = set
                    .songs
                    .iter()
                    .position(|s| &s.name == song)
                    .ok_or_else(|| format!("there's no song `{song}`"))?;
                if set.songs[k].kind != Kind::Music {
                    return Err(format!("`{song}` is an effect, not music"));
                }
                for s in &mut set.songs {
                    if level.is_some() && s.level == *level {
                        s.level = None;
                    }
                    if state.is_some() && s.state == *state {
                        s.state = None;
                    }
                }
                if level.is_some() {
                    set.songs[k].level = level.clone();
                }
                if state.is_some() {
                    set.songs[k].state = state.clone();
                }
                said.push(format!(
                    "{song} plays in {}",
                    level.clone().or(state.clone()).unwrap_or_default()
                ));
            }
        }
    }
    set.check()?;
    Ok((set, map, said))
}

/// Whether a song is ready to edit: the model knows it completely. Returns
/// what's missing.
pub fn readiness(set: &SongSet, song: &str) -> Vec<String> {
    let mut miss = Vec::new();
    let Some(s) = set.song(song) else {
        return vec![format!("there's no song `{song}`")];
    };
    if s.tracks.is_empty() {
        miss.push("no channel was captured".into());
    }
    if s.confidence < 0.8 {
        miss.push(format!(
            "capture is {:.0}% sure of it (an effect took channels while it played, or no loop was found)",
            s.confidence * 100.0
        ));
    }
    for t in &s.tracks {
        for p in t.placed(s.tempo) {
            let ins = &set.instruments[usize::from(p.note.instrument)];
            if ins.role.is_none() {
                miss.push(format!("instrument `{}` has no role", ins.name));
                break;
            }
        }
    }
    match s.kind {
        Kind::Music if s.level.is_none() && s.state.is_none() => {
            miss.push("it isn't tied to a level or a state".into())
        }
        Kind::Sfx if s.event.is_none() => miss.push("it isn't tied to a model event".into()),
        _ => {}
    }
    if s.kind == Kind::Music && s.loop_frame.is_none() {
        miss.push("its loop isn't known".into());
    }
    miss.dedup();
    miss
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apu::Region;
    use crate::capture::{self, Options};

    fn set() -> SongSet {
        let log = crate::tests::made_up_log(2, None);
        capture::capture(
            Region::Ntsc,
            &log.states(),
            &[],
            &Options {
                name: "made up".into(),
                level: Some("one".into()),
                ..Options::default()
            },
        )
    }

    #[test]
    fn ops_change_what_they_say_and_refuse_the_rest() {
        let s = set();
        let m = Mapping::default();
        let (t, m2, said) = apply(
            &s,
            &m,
            &[
                Op::Transpose {
                    song: "made_up".into(),
                    semitones: 2,
                    channel: Some("pulse1".into()),
                },
                Op::Tempo {
                    song: "made_up".into(),
                    frames_per_row: 32,
                },
                Op::Mute {
                    channels: vec!["noise".into()],
                },
                Op::Remap {
                    instrument: None,
                    role: Some("bass".into()),
                    song: None,
                    target: Target::Gm { program: 33 },
                    gain: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(said.len(), 4);
        let a = s.songs[0]
            .track(Channel::Pulse1)
            .unwrap()
            .placed(s.songs[0].tempo);
        let b = t.songs[0]
            .track(Channel::Pulse1)
            .unwrap()
            .placed(t.songs[0].tempo);
        assert_eq!(b[0].note.pitch, a[0].note.pitch + 2);
        assert_eq!(b[1].start, a[1].start * 2);
        assert_eq!(t.songs[0].frames, s.songs[0].frames * 2);
        assert_eq!(m2.mix.gains()[3], 0.0);
        assert!(
            apply(
                &s,
                &m,
                &[Op::Transpose {
                    song: "made_up".into(),
                    semitones: 100,
                    channel: None
                }]
            )
            .is_err()
        );
        assert!(
            apply(
                &s,
                &m,
                &[Op::Mute {
                    channels: vec!["kazoo".into()]
                }]
            )
            .is_err()
        );
        assert!(
            apply(
                &s,
                &m,
                &[Op::Swap {
                    song: "nope".into(),
                    level: Some("one".into()),
                    state: None
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn readiness_names_what_is_missing() {
        let s = set();
        assert!(
            readiness(&s, "made_up").is_empty(),
            "{:?}",
            readiness(&s, "made_up")
        );
        let mut s2 = s.clone();
        s2.songs[0].level = None;
        assert!(readiness(&s2, "made_up")[0].contains("level"));
    }
}
