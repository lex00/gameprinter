//! The player: a sound driver of our own that turns songs and effects into
//! APU register writes, a frame at a time, for the synthesizer. Music
//! loops; an effect takes the channels it uses until it ends, as NES
//! drivers give effects priority. It is deterministic and keeps all its
//! state in itself, so every runtime that steps it once a frame writes the
//! same registers.

use crate::apu::{Channel, Region};
use crate::song::{Placed, Song, SongSet};

#[derive(Debug, Clone)]
struct Cursor {
    song: usize,
    frame: u64,
    tracks: [Vec<Placed>; 5],
    looping: bool,
}

impl Cursor {
    fn new(set: &SongSet, song: usize, looping: bool) -> Self {
        let s = &set.songs[song];
        let mut tracks: [Vec<Placed>; 5] = Default::default();
        for t in &s.tracks {
            tracks[t.channel.index()] = t.placed(s.tempo);
        }
        Self {
            song,
            frame: 0,
            tracks,
            looping,
        }
    }

    fn uses(&self, c: Channel) -> bool {
        !self.tracks[c.index()].is_empty()
    }

    /// The note sounding now on `c`, and whether it starts this frame.
    fn note(&self, c: Channel) -> Option<(&Placed, bool)> {
        let t = &self.tracks[c.index()];
        let i = t.partition_point(|p| p.start <= self.frame);
        let p = t[..i].last()?;
        (self.frame < p.start + u64::from(p.note.len)).then_some((p, p.start == self.frame))
    }
}

/// What the player writes for one frame: (cycle, register, value).
pub type Writes = Vec<(u32, u8, u8)>;

#[derive(Debug, Clone)]
pub struct Player {
    pub set: SongSet,
    music: Option<Cursor>,
    effects: Vec<Cursor>,
    /// What last played on each channel: (cursor identity, note start),
    /// and the last high period byte written.
    last: [Option<(usize, bool, u64)>; 5],
    hi: [Option<u8>; 5],
    started: bool,
    /// Channels muted by the mix.
    pub muted: [bool; 5],
}

impl Player {
    pub fn new(set: SongSet) -> Self {
        Self {
            set,
            music: None,
            effects: Vec::new(),
            last: [None; 5],
            hi: [None; 5],
            started: false,
            muted: [false; 5],
        }
    }

    pub fn region(&self) -> Region {
        self.set.region
    }

    /// Starts a song by name from its start; `None` stops the music. A
    /// song that's already playing carries on.
    pub fn play_music(&mut self, name: Option<&str>) {
        let i = name.and_then(|n| self.set.songs.iter().position(|s| s.name == n));
        if self.music.as_ref().map(|c| c.song) == i {
            return;
        }
        self.music = i.map(|i| Cursor::new(&self.set, i, true));
    }

    /// Starts an effect by name.
    pub fn play_effect(&mut self, name: &str) {
        if let Some(i) = self.set.songs.iter().position(|s| s.name == name) {
            self.effects.retain(|c| c.song != i);
            self.effects.push(Cursor::new(&self.set, i, false));
        }
    }

    pub fn music(&self) -> Option<&Song> {
        self.music.as_ref().map(|c| &self.set.songs[c.song])
    }

    /// The writes for the next frame.
    pub fn frame(&mut self) -> Writes {
        let mut w: Writes = Vec::new();
        if !self.started {
            w.push((0, 0x15, 0x0F));
            w.push((0, 0x17, 0x40));
            self.started = true;
        }
        for c in Channel::ALL {
            // The newest effect using the channel, else the music.
            let src = self
                .effects
                .iter()
                .enumerate()
                .rev()
                .find(|(_, e)| e.uses(c))
                .map(|(i, e)| (i, true, e))
                .or_else(|| self.music.as_ref().map(|m| (usize::MAX, false, m)));
            let note = src.and_then(|(i, fx, cur)| {
                cur.note(c)
                    .map(|(p, starts)| (i, fx, cur.song, p.clone(), starts))
            });
            match note {
                Some((i, fx, song, p, starts)) if !self.muted[c.index()] => {
                    let id = (i, fx, p.start);
                    let starts = starts || self.last[c.index()] != Some(id);
                    self.last[c.index()] = Some(id);
                    let k = (self.cursor_frame(i, fx) - p.start) as usize;
                    self.write_note(&mut w, c, song, &p, k, starts);
                }
                _ => {
                    self.last[c.index()] = None;
                    Self::silence(&mut w, c);
                }
            }
        }
        // Advance.
        if let Some(m) = &mut self.music {
            m.frame += 1;
            let s = &self.set.songs[m.song];
            if m.frame >= s.frames {
                match (m.looping, s.loop_frame) {
                    (true, Some(l)) if l < s.frames => m.frame = l,
                    _ => self.music = None,
                }
            }
        }
        for e in &mut self.effects {
            e.frame += 1;
        }
        let songs = &self.set.songs;
        self.effects.retain(|e| e.frame < songs[e.song].frames);
        w
    }

    fn cursor_frame(&self, i: usize, fx: bool) -> u64 {
        if fx {
            self.effects[i].frame
        } else {
            self.music.as_ref().map_or(0, |m| m.frame)
        }
    }

    fn silence(w: &mut Writes, c: Channel) {
        match c {
            Channel::Pulse1 => w.push((0, 0x00, 0x30)),
            Channel::Pulse2 => w.push((0, 0x04, 0x30)),
            Channel::Triangle => w.push((0, 0x08, 0x80)),
            Channel::Noise => w.push((0, 0x0C, 0x30)),
            Channel::Dmc => {}
        }
    }

    fn write_note(
        &mut self,
        w: &mut Writes,
        c: Channel,
        _song: usize,
        p: &Placed,
        k: usize,
        starts: bool,
    ) {
        let region = self.set.region;
        let Some(ins) = self.set.instruments.get(usize::from(p.note.instrument)) else {
            Self::silence(w, c);
            return;
        };
        let vol = ins.volume_at(k) & 0x0F;
        let duty = ins.duty_at(k) & 3;
        let base = i32::from(p.note.period(c, region));
        let period = (base + i32::from(ins.pitch_at(k))).clamp(0, 0x7FF) as u16;
        let lo = (period & 0xFF) as u8;
        let hi = ((period >> 8) & 7) as u8;
        let ci = c.index();
        match c {
            Channel::Pulse1 | Channel::Pulse2 => {
                let r = if c == Channel::Pulse1 { 0 } else { 4 };
                w.push((0, r, (duty << 6) | 0x30 | vol));
                w.push((0, r + 1, 0x08));
                w.push((0, r + 2, lo));
                if starts || self.hi[ci] != Some(hi) {
                    w.push((0, r + 3, hi | 0x08));
                    self.hi[ci] = Some(hi);
                }
            }
            Channel::Triangle => {
                if vol == 0 {
                    w.push((0, 0x08, 0x80));
                } else {
                    w.push((0, 0x08, 0xFF));
                }
                w.push((0, 0x0A, lo));
                if starts || self.hi[ci] != Some(hi) {
                    w.push((0, 0x0B, hi | 0x08));
                    self.hi[ci] = Some(hi);
                }
            }
            Channel::Noise => {
                let idx = (period & 0x0F) as u8;
                w.push((0, 0x0C, 0x30 | vol));
                w.push((0, 0x0E, ((duty & 1) << 7) | idx));
                if starts {
                    w.push((0, 0x0F, 0x08));
                }
            }
            Channel::Dmc => {
                if starts && let Some([addr, len]) = ins.sample {
                    w.push((0, 0x10, p.note.pitch & 0x0F));
                    w.push((0, 0x12, ((addr.wrapping_sub(0xC000)) >> 6) as u8));
                    w.push((0, 0x13, (len.saturating_sub(1) >> 4) as u8));
                    w.push((0, 0x15, 0x0F));
                    w.push((0, 0x15, 0x1F));
                }
            }
        }
    }
}

/// One song played from its start for its length (no loop) as a timeline
/// of writes at absolute CPU cycles, and the frame count.
pub fn song_timeline(set: &SongSet, name: &str) -> Option<(Vec<(u64, u8, u8)>, u64)> {
    let s = set.song(name)?;
    let frames = s.frames;
    let mut p = Player::new(set.clone());
    let i = set.songs.iter().position(|x| x.name == name)?;
    p.music = Some(Cursor::new(&p.set, i, false));
    let mut out = Vec::new();
    for f in 0..frames {
        let start = set.region.frame_start(f);
        for (c, r, v) in p.frame() {
            out.push((start + u64::from(c), r, v));
        }
    }
    Some((out, frames))
}
