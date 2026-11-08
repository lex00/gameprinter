//! Songs and sound effects as notes (docs/formats/songs.md): the general
//! representation every source lands in. A captured register log, a
//! decoded driver format, a MIDI or FamiStudio import and an agent's edit
//! all produce a [`SongSet`]; the player, the synthesizer, the exporters
//! and the model's `songs` slots read one.
//!
//! A song has a track per NES channel. A track is patterns placed in order;
//! a pattern holds notes timed in frames from its start, so a song keeps
//! its exact timing whatever its tempo grid. Instruments are envelopes per
//! frame (volume, duty and a pitch offset in timer units), shared by every
//! song of a set. Each song says where it came from and how sure capture
//! is of it.

use serde::{Deserialize, Serialize};

use crate::apu::{Channel, Region, Sample, period_of};

/// The file format's name and version.
pub const FORMAT: &str = "gp-songs";
pub const VERSION: &str = "0.1.0";

/// What a channel's instrument drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Voice {
    Pulse,
    Triangle,
    Noise,
    Dmc,
}

impl Voice {
    pub fn of(c: Channel) -> Self {
        match c {
            Channel::Pulse1 | Channel::Pulse2 => Self::Pulse,
            Channel::Triangle => Self::Triangle,
            Channel::Noise => Self::Noise,
            Channel::Dmc => Self::Dmc,
        }
    }
}

/// Where a song's notes came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// Decoded from the game's own music data by a known driver's decoder.
    Driver,
    /// Derived from a register log of a recording.
    Log,
    /// Taken from rendered audio (none yet: filed).
    Samples,
    /// Imported from an external editor's file (FamiStudio, MIDI).
    Import,
    /// Written or edited by hand or by an agent.
    Edited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Music,
    Sfx,
}

/// An instrument: what a note does frame by frame from its start. Each
/// envelope's last value holds for the rest of a longer note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instrument {
    pub name: String,
    pub voice: Voice,
    /// What it plays in the music (`lead`, `bass`, `harmony`, `drums`,
    /// `sfx`), for sound banks and themes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Volume, 0-15, a frame each.
    pub volume: Vec<u8>,
    /// Pulse duty 0-3, or the noise mode (1: the short loop).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub duty: Vec<u8>,
    /// Timer period offset from the note's own, a frame each: vibrato,
    /// slides and sweeps. For noise, an offset of the period index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pitch: Vec<i16>,
    /// A DMC instrument's sample: its address and length in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<[u16; 2]>,
    /// How many captured notes it was clustered from.
    #[serde(default)]
    pub notes: u32,
}

impl Instrument {
    fn at<T: Copy + Default>(v: &[T], k: usize) -> T {
        v.get(k).or(v.last()).copied().unwrap_or_default()
    }
    pub fn volume_at(&self, k: usize) -> u8 {
        if self.volume.is_empty() {
            15
        } else {
            Self::at(&self.volume, k)
        }
    }
    pub fn duty_at(&self, k: usize) -> u8 {
        Self::at(&self.duty, k)
    }
    pub fn pitch_at(&self, k: usize) -> i16 {
        Self::at(&self.pitch, k)
    }
}

/// A note: a pitch held for a number of frames with an instrument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// Frames from the pattern's start.
    pub at: u32,
    /// Frames it sounds.
    pub len: u32,
    /// A MIDI note number for pulse and triangle (60 is middle C); the
    /// period index 0-15 for noise; the rate index 0-15 for the DMC.
    pub pitch: u8,
    /// The timer period's offset from the pitch's nearest period, so a
    /// captured note plays at exactly the period the game used.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fine: i16,
    /// Index into the set's instruments.
    pub instrument: u16,
}

fn is_zero(v: &i16) -> bool {
    *v == 0
}

impl Note {
    /// The timer period it starts at on `channel`.
    pub fn period(&self, channel: Channel, region: Region) -> u16 {
        match channel {
            Channel::Noise | Channel::Dmc => u16::from(self.pitch & 0x0F),
            _ => (i32::from(period_of(channel, f64::from(self.pitch), region))
                + i32::from(self.fine))
            .clamp(0, 0x7FF) as u16,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern {
    pub notes: Vec<Note>,
}

/// One channel's part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub channel: Channel,
    pub patterns: Vec<Pattern>,
    /// Pattern indices in play order, one per pattern-length column; an
    /// empty column is `null`.
    pub order: Vec<Option<usize>>,
}

/// Rows and patterns, in frames: the grid an editor shows. Notes keep
/// their own frames, so the grid never changes what plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tempo {
    pub frames_per_row: u32,
    pub rows_per_pattern: u32,
}

impl Tempo {
    pub fn pattern_frames(&self) -> u32 {
        self.frames_per_row * self.rows_per_pattern
    }
}

/// The game's request that started a sound, as gp gaps found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub via: String,
    pub id: u8,
    pub name: String,
}

/// Where a capture came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Captured {
    pub recording: String,
    /// The frames the song was taken from, end excluded.
    pub frames: [u64; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Song {
    pub name: String,
    pub kind: Kind,
    pub origin: Origin,
    /// How sure the source is that these are the game's notes, 0 to 1.
    pub confidence: f64,
    /// The driver whose decoder read it, for `origin = "driver"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    /// Frames from the start to the end (of the loop, when it loops).
    pub frames: u64,
    /// The frame the loop goes back to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_frame: Option<u64>,
    pub tempo: Tempo,
    pub tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Request>,
    /// For an effect: the model event it marks (`enemy.stomped`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// For music: the level it plays in, or the rule state it plays in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured: Option<Captured>,
    /// What capture couldn't settle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// A note placed in song time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub start: u64,
    pub note: Note,
}

impl Track {
    /// Its notes in song time, in order.
    pub fn placed(&self, tempo: Tempo) -> Vec<Placed> {
        let pf = u64::from(tempo.pattern_frames());
        let mut out = Vec::new();
        for (col, p) in self.order.iter().enumerate() {
            let Some(p) = p.and_then(|i| self.patterns.get(i)) else {
                continue;
            };
            for n in &p.notes {
                out.push(Placed {
                    start: col as u64 * pf + u64::from(n.at),
                    note: n.clone(),
                });
            }
        }
        out.sort_by_key(|p| p.start);
        out
    }

    /// A track from notes in song time: cut into pattern-length columns,
    /// identical columns sharing a pattern. A note longer than the rest of
    /// its column keeps its length (the player holds it).
    pub fn from_placed(channel: Channel, notes: &[Placed], tempo: Tempo, frames: u64) -> Self {
        let pf = u64::from(tempo.pattern_frames().max(1));
        let cols = frames.div_ceil(pf).max(1) as usize;
        let mut patterns: Vec<Pattern> = Vec::new();
        let mut order = Vec::with_capacity(cols);
        for col in 0..cols {
            let lo = col as u64 * pf;
            let notes: Vec<Note> = notes
                .iter()
                .filter(|p| p.start >= lo && p.start < lo + pf)
                .map(|p| Note {
                    at: (p.start - lo) as u32,
                    ..p.note.clone()
                })
                .collect();
            if notes.is_empty() {
                order.push(None);
                continue;
            }
            let pat = Pattern { notes };
            let i = match patterns.iter().position(|q| *q == pat) {
                Some(i) => i,
                None => {
                    patterns.push(pat);
                    patterns.len() - 1
                }
            };
            order.push(Some(i));
        }
        Self {
            channel,
            patterns,
            order,
        }
    }
}

impl Song {
    pub fn track(&self, c: Channel) -> Option<&Track> {
        self.tracks.iter().find(|t| t.channel == c)
    }

    pub fn note_count(&self) -> usize {
        self.tracks.iter().map(|t| t.placed(self.tempo).len()).sum()
    }

    /// Re-cuts every track on a new grid; what plays doesn't change.
    pub fn regrid(&mut self, tempo: Tempo) {
        let old = self.tempo;
        for t in &mut self.tracks {
            let placed = t.placed(old);
            *t = Track::from_placed(t.channel, &placed, tempo, self.frames);
        }
        self.tempo = tempo;
    }

    /// Channels it plays on.
    pub fn channels(&self) -> Vec<Channel> {
        self.tracks
            .iter()
            .filter(|t| t.order.iter().any(Option::is_some))
            .map(|t| t.channel)
            .collect()
    }
}

/// Songs, effects and the instruments they share.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SongSet {
    pub format: String,
    pub version: String,
    pub region: Region,
    pub instruments: Vec<Instrument>,
    pub songs: Vec<Song>,
    /// DMC samples' bytes, when capture read them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<Sample>,
}

impl SongSet {
    pub fn new(region: Region) -> Self {
        Self {
            format: FORMAT.into(),
            version: VERSION.into(),
            region,
            instruments: Vec::new(),
            songs: Vec::new(),
            samples: Vec::new(),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let s: SongSet = serde_json::from_str(text).map_err(|e| format!("songs: {e}"))?;
        if s.format != FORMAT {
            return Err(format!("not a {FORMAT} file: format `{}`", s.format));
        }
        let ok = s
            .version
            .rsplit_once('.')
            .is_some_and(|(mm, _)| VERSION.starts_with(&format!("{mm}.")));
        if !ok {
            return Err(format!(
                "{FORMAT} {} isn't readable by this reader ({VERSION})",
                s.version
            ));
        }
        s.check()?;
        Ok(s)
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string_pretty(self).expect("songs serialize") + "\n"
    }

    /// Every note names an instrument that exists and fits its channel.
    pub fn check(&self) -> Result<(), String> {
        for s in &self.songs {
            for t in &s.tracks {
                for p in &t.patterns {
                    for n in &p.notes {
                        let Some(i) = self.instruments.get(usize::from(n.instrument)) else {
                            return Err(format!(
                                "{}: a note on {} names instrument {}, which doesn't exist",
                                s.name,
                                t.channel.name(),
                                n.instrument
                            ));
                        };
                        if i.voice != Voice::of(t.channel) {
                            return Err(format!(
                                "{}: instrument `{}` is a {:?} instrument on {}",
                                s.name,
                                i.name,
                                i.voice,
                                t.channel.name()
                            ));
                        }
                    }
                }
                if t.order.iter().flatten().any(|&i| i >= t.patterns.len()) {
                    return Err(format!(
                        "{}: {} plays a pattern that doesn't exist",
                        s.name,
                        t.channel.name()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Several sets as one, each song named `<prefix>/<name>` and each
    /// set's instruments and samples kept apart: what a player needs to
    /// play songs and effects from several slots. The first set's region.
    pub fn merged(sets: &[(String, &SongSet)]) -> SongSet {
        let mut out = SongSet::new(sets.first().map_or_else(Region::default, |s| s.1.region));
        for (prefix, set) in sets {
            let base = out.instruments.len() as u16;
            out.instruments.extend(set.instruments.iter().cloned());
            for smp in &set.samples {
                if !out.samples.iter().any(|x| x.addr == smp.addr) {
                    out.samples.push(smp.clone());
                }
            }
            for song in &set.songs {
                let mut s = song.clone();
                s.name = format!("{prefix}/{}", song.name);
                for t in &mut s.tracks {
                    for p in &mut t.patterns {
                        for n in &mut p.notes {
                            n.instrument += base;
                        }
                    }
                }
                out.songs.push(s);
            }
        }
        out
    }

    pub fn song(&self, name: &str) -> Option<&Song> {
        self.songs.iter().find(|s| s.name == name)
    }

    pub fn song_mut(&mut self, name: &str) -> Option<&mut Song> {
        self.songs.iter_mut().find(|s| s.name == name)
    }
}
