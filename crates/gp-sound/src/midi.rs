//! Standard MIDI files (type 1), the general interchange: export from the
//! note representation and import back with a check against the NES's
//! channels (docs/sound.md, "MIDI").
//!
//! Export: track 0 holds the tempo and the song's name; then one track per
//! NES channel, named after it. A frame is one tick, and a quarter note is
//! four rows of the song's grid, so the division is `4 * frames_per_row`
//! ticks and the tempo the matching microseconds. Pulse and triangle play
//! on MIDI channels 1 to 3 with a General MIDI program by the instrument's
//! role (or the sound bank mapping's `gm` program), noise and DMC on
//! channel 10 with the drum map. A note's velocity is its instrument's
//! first volume; volume envelopes are expression (CC 11) changes and pitch
//! envelopes pitch bends, with the bend range set to 12 semitones.
//!
//! Import: tempo from the first tempo event, notes from note on and off,
//! melodic channels to pulse 1, pulse 2 and the triangle in order of first
//! note (or by track name), channel 10 to noise. One note at a time per
//! channel: a chord keeps its highest note, with a warning. Pitches out of
//! a channel's range are errors.

use std::collections::BTreeMap;

use crate::apu::{Channel, Region};
use crate::famistudio::Validation;
use crate::song::{Instrument, Kind, Note, Origin, Placed, Song, SongSet, Tempo, Track, Voice};

/// A General MIDI program (0-based) for a role.
pub fn gm_program(role: Option<&str>, voice: Voice) -> u8 {
    match (role, voice) {
        (Some("bass"), _) | (None, Voice::Triangle) => 38, // Synth Bass 1
        (Some("harmony"), _) => 81,                        // Lead 2 (sawtooth)
        (Some("sfx"), _) => 80,
        _ => 80, // Lead 1 (square)
    }
}

/// A noise note's General MIDI drum key: high periods (low index) are
/// hats, the middle snares, the low end kicks.
pub fn drum_key(index: u8, len: u32, mode: u8) -> u8 {
    if mode == 1 {
        return 56; // Cowbell: the short, metallic loop
    }
    match index {
        0..=5 if len > 8 => 46, // Open hi-hat
        0..=5 => 42,            // Closed hi-hat
        6..=10 => 38,           // Acoustic snare
        _ => 36,                // Bass drum 1
    }
}

fn drum_index(key: u8) -> u8 {
    match key {
        42 | 44 | 46 | 49 | 51 | 57 => 3,
        37..=40 => 8,
        35 | 36 => 12,
        _ => 6,
    }
}

fn vlq(mut v: u32, out: &mut Vec<u8>) {
    let mut buf = [0u8; 5];
    let mut i = 4;
    buf[i] = (v & 0x7F) as u8;
    v >>= 7;
    while v > 0 {
        i -= 1;
        buf[i] = 0x80 | (v & 0x7F) as u8;
        v >>= 7;
    }
    out.extend_from_slice(&buf[i..]);
}

fn chunk(kind: &[u8; 4], body: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
}

/// Events at ticks, sorted then written with delta times.
fn track_bytes(mut ev: Vec<(u64, u8, Vec<u8>)>) -> Vec<u8> {
    // Order within a tick: offs (0) before everything else (1).
    ev.sort_by_key(|e| (e.0, e.1));
    let mut b = Vec::new();
    let mut t = 0;
    for (at, _, bytes) in ev {
        vlq((at - t) as u32, &mut b);
        b.extend_from_slice(&bytes);
        t = at;
    }
    b.extend_from_slice(&[0, 0xFF, 0x2F, 0]);
    b
}

fn meta_text(kind: u8, s: &str) -> Vec<u8> {
    let mut v = vec![0xFF, kind];
    vlq(s.len() as u32, &mut v);
    v.extend_from_slice(s.as_bytes());
    v
}

/// The GM program a mapping gives an instrument, if any.
pub type Programs = BTreeMap<String, u8>;

/// Writes one song as a type 1 Standard MIDI file.
pub fn export(set: &SongSet, song: &Song, programs: &Programs) -> Vec<u8> {
    let fpr = song.tempo.frames_per_row.max(1);
    let division = (4 * fpr) as u16;
    let us_per_quarter =
        (f64::from(division) * 1_000_000.0 / set.region.frame_rate()).round() as u32;
    let mut out = Vec::new();
    let mut hd = Vec::new();
    hd.extend_from_slice(&1u16.to_be_bytes());
    let tracks: Vec<&Track> = song.tracks.iter().collect();
    hd.extend_from_slice(&((tracks.len() + 1) as u16).to_be_bytes());
    hd.extend_from_slice(&division.to_be_bytes());
    chunk(b"MThd", &hd, &mut out);
    let mut t0 = vec![(0, 1, meta_text(0x03, &song.name))];
    let tb = us_per_quarter.to_be_bytes();
    t0.push((0, 1, vec![0xFF, 0x51, 3, tb[1], tb[2], tb[3]]));
    t0.push((0, 1, vec![0xFF, 0x58, 4, 4, 2, 24, 8]));
    if let Some(l) = song.loop_frame {
        t0.push((l, 1, meta_text(0x06, "loop")));
    }
    t0.push((song.frames, 1, meta_text(0x06, "end")));
    chunk(b"MTrk", &track_bytes(t0), &mut out);
    let mut melodic = 0u8;
    for t in tracks {
        let ch = match t.channel {
            Channel::Noise | Channel::Dmc => 9u8,
            _ => {
                melodic += 1;
                melodic - 1
            }
        };
        let mut ev: Vec<(u64, u8, Vec<u8>)> = vec![(0, 1, meta_text(0x03, t.channel.name()))];
        // Bend range 12 semitones.
        for (c, v) in [(101u8, 0u8), (100, 0), (6, 12), (38, 0)] {
            ev.push((0, 1, vec![0xB0 | ch, c, v]));
        }
        let mut program: Option<u8> = None;
        for p in t.placed(song.tempo) {
            let ins = &set.instruments[usize::from(p.note.instrument)];
            let start = p.start;
            let end = start + u64::from(p.note.len.max(1));
            let key = match t.channel {
                Channel::Noise => drum_key(p.note.pitch, p.note.len, ins.duty_at(0)),
                Channel::Dmc => 35,
                _ => p.note.pitch.min(127),
            };
            if ch != 9 {
                let want = programs
                    .get(&ins.name)
                    .copied()
                    .unwrap_or_else(|| gm_program(ins.role.as_deref(), ins.voice));
                if program != Some(want) {
                    ev.push((start, 1, vec![0xC0 | ch, want]));
                    program = Some(want);
                }
            }
            if t.channel == Channel::Dmc
                && let Some([a, l]) = ins.sample
            {
                ev.push((
                    start,
                    1,
                    meta_text(0x01, &format!("dmc sample {a:04x} {l}")),
                ));
            }
            let vel = |v: u8| (v.min(15) * 8 + 7).min(127);
            ev.push((start, 1, vec![0x90 | ch, key, vel(ins.volume_at(0)).max(1)]));
            if ch != 9 {
                let mut last_vol = ins.volume_at(0);
                let mut last_bend = 8192u16;
                ev.push((start, 1, vec![0xB0 | ch, 11, 127]));
                let base = p.note.period(t.channel, set.region);
                for k in 0..u64::from(p.note.len) {
                    let v = ins.volume_at(k as usize);
                    if k > 0 && v != last_vol {
                        let e = (u32::from(v) * 127 / u32::from(ins.volume_at(0).max(1))).min(127);
                        ev.push((start + k, 1, vec![0xB0 | ch, 11, e as u8]));
                        last_vol = v;
                    }
                    let d = ins.pitch_at(k as usize);
                    let bend = if d == 0 {
                        8192
                    } else {
                        let pb = f64::from(base);
                        let pn = (pb + f64::from(d)).max(1.0);
                        let semis = 12.0 * ((pb + 1.0) / (pn + 1.0)).log2();
                        (8192.0 + semis / 12.0 * 8192.0).round().clamp(0.0, 16383.0) as u16
                    };
                    if bend != last_bend {
                        ev.push((
                            start + k,
                            1,
                            vec![0xE0 | ch, (bend & 0x7F) as u8, (bend >> 7) as u8],
                        ));
                        last_bend = bend;
                    }
                }
                if last_bend != 8192 {
                    ev.push((end, 1, vec![0xE0 | ch, 0, 64]));
                }
            }
            ev.push((end, 0, vec![0x80 | ch, key, 0]));
        }
        chunk(b"MTrk", &track_bytes(ev), &mut out);
    }
    out
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Result<u8, String> {
        let v = *self.b.get(self.i).ok_or("the file ends early")?;
        self.i += 1;
        Ok(v)
    }
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self
            .b
            .get(self.i..self.i + n)
            .ok_or("the file ends early")?;
        self.i += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn vlq(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..4 {
            let b = self.u8()?;
            v = (v << 7) | u32::from(b & 0x7F);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("a variable-length number longer than 4 bytes".into())
    }
}

/// A MIDI note: tick on, tick off, channel, key, velocity, program, and
/// the track's name.
#[derive(Debug, Clone)]
struct MNote {
    on: u64,
    off: u64,
    ch: u8,
    key: u8,
    vel: u8,
    program: u8,
    track: Option<String>,
}

/// Reads a Standard MIDI file (type 0 or 1) as one song.
pub fn import(bytes: &[u8], name: &str, region: Region) -> (SongSet, Validation) {
    let mut v = Validation::default();
    let mut set = SongSet::new(region);
    let parsed = (|| -> Result<(u16, Vec<MNote>, Option<u32>, Option<u64>), String> {
        let mut r = Reader { b: bytes, i: 0 };
        if r.take(4)? != b"MThd" {
            return Err("not a Standard MIDI file".into());
        }
        let hl = r.u32()? as usize;
        let hdr = r.take(hl)?.to_vec();
        if hdr.len() < 6 {
            return Err("a short MIDI header".into());
        }
        let format = u16::from_be_bytes([hdr[0], hdr[1]]);
        let ntracks = u16::from_be_bytes([hdr[2], hdr[3]]);
        let division = u16::from_be_bytes([hdr[4], hdr[5]]);
        if format > 1 {
            return Err(format!("MIDI format {format} isn't read (0 and 1 are)"));
        }
        if division & 0x8000 != 0 {
            return Err("SMPTE time divisions aren't read".into());
        }
        let mut notes = Vec::new();
        let mut tempo = None;
        let mut loop_tick = None;
        for _ in 0..ntracks {
            if r.take(4)? != b"MTrk" {
                return Err("a track chunk is missing".into());
            }
            let len = r.u32()? as usize;
            let end = r.i + len;
            let mut t = 0u64;
            let mut status = 0u8;
            let mut open: BTreeMap<(u8, u8), (u64, u8, u8)> = BTreeMap::new();
            let mut program = [0u8; 16];
            let mut tname = None;
            while r.i < end {
                t += u64::from(r.vlq()?);
                let mut b = r.u8()?;
                if b < 0x80 {
                    r.i -= 1;
                    b = status;
                } else if b < 0xF0 {
                    status = b;
                }
                match b {
                    0xFF => {
                        let kind = r.u8()?;
                        let l = r.vlq()? as usize;
                        let data = r.take(l)?.to_vec();
                        match kind {
                            0x51 if data.len() == 3 => {
                                let us = u32::from_be_bytes([0, data[0], data[1], data[2]]);
                                if tempo.is_some_and(|x| x != us) {
                                    return Err("tempo changes aren't read; give one tempo".into());
                                }
                                tempo = Some(us);
                            }
                            0x03 => tname = Some(String::from_utf8_lossy(&data).into_owned()),
                            0x06 if data == b"loop" => loop_tick = Some(t),
                            _ => {}
                        }
                    }
                    0xF0 | 0xF7 => {
                        let l = r.vlq()? as usize;
                        r.take(l)?;
                    }
                    _ => {
                        let ch = b & 0x0F;
                        match b & 0xF0 {
                            0x90 | 0x80 => {
                                let key = r.u8()?;
                                let vel = r.u8()?;
                                if b & 0xF0 == 0x90 && vel > 0 {
                                    open.insert((ch, key), (t, vel, program[usize::from(ch)]));
                                } else if let Some((on, vel, prog)) = open.remove(&(ch, key)) {
                                    notes.push(MNote {
                                        on,
                                        off: t,
                                        ch,
                                        key,
                                        vel,
                                        program: prog,
                                        track: tname.clone(),
                                    });
                                }
                            }
                            0xC0 => program[usize::from(ch)] = r.u8()?,
                            0xD0 => {
                                r.u8()?;
                            }
                            0xA0 | 0xB0 | 0xE0 => {
                                r.take(2)?;
                            }
                            _ => return Err(format!("a MIDI status byte {b:#04x} out of place")),
                        }
                    }
                }
            }
            r.i = end;
        }
        Ok((division, notes, tempo, loop_tick))
    })();
    let (division, mut notes, tempo, loop_tick) = match parsed {
        Ok(x) => x,
        Err(e) => {
            v.errors.push(e);
            return (set, v);
        }
    };
    let us = tempo.unwrap_or(500_000);
    let rate = region.frame_rate();
    let frame_of = |tick: u64| -> u64 {
        (tick as f64 * f64::from(us) / f64::from(division.max(1)) * rate / 1_000_000.0).round()
            as u64
    };
    // NES channels: by track name, else melodic MIDI channels in order.
    notes.sort_by_key(|n| (n.on, n.ch, std::cmp::Reverse(n.key)));
    let mut assign: BTreeMap<u8, Channel> = BTreeMap::new();
    let mut free = vec![Channel::Pulse1, Channel::Pulse2, Channel::Triangle];
    for n in &notes {
        if assign.contains_key(&n.ch) {
            continue;
        }
        let named = n.track.as_deref().and_then(Channel::parse);
        let c = if n.ch == 9 {
            Some(
                named
                    .filter(|c| *c == Channel::Dmc)
                    .unwrap_or(Channel::Noise),
            )
        } else if let Some(c) = named.filter(|c| free.contains(c)) {
            free.retain(|x| *x != c);
            Some(c)
        } else if free.is_empty() {
            None
        } else {
            Some(free.remove(0))
        };
        match c {
            Some(c) => {
                assign.insert(n.ch, c);
            }
            None => v.errors.push(format!(
                "MIDI channel {} is a fourth melodic part; the NES has two pulses and a triangle",
                n.ch + 1
            )),
        }
    }
    let fpr = (u64::from(division) / 4).max(1);
    let fpr_frames = frame_of(fpr).max(1) as u32;
    let tempo = Tempo {
        frames_per_row: fpr_frames,
        rows_per_pattern: 16,
    };
    let mut per: BTreeMap<Channel, Vec<Placed>> = BTreeMap::new();
    let mut ins_of: BTreeMap<(Channel, u8, u8), u16> = BTreeMap::new();
    let mut end = 0u64;
    for n in &notes {
        let Some(&c) = assign.get(&n.ch) else {
            continue;
        };
        let start = frame_of(n.on);
        let stop = frame_of(n.off).max(start + 1);
        end = end.max(stop);
        let vol = (n.vel / 8).clamp(1, 15);
        let (pitch, prog) = match c {
            Channel::Noise | Channel::Dmc => (drum_index(n.key), 0),
            _ => (n.key, n.program),
        };
        let ki = (c, prog, vol);
        let voice = Voice::of(c);
        let i = *ins_of.entry(ki).or_insert_with(|| {
            set.instruments.push(Instrument {
                name: format!("{}-p{prog}-v{vol}", c.name()),
                voice,
                role: None,
                volume: vec![vol],
                duty: if voice == Voice::Pulse {
                    vec![2]
                } else {
                    Vec::new()
                },
                pitch: Vec::new(),
                sample: None,
                notes: 0,
            });
            (set.instruments.len() - 1) as u16
        });
        set.instruments[usize::from(i)].notes += 1;
        let list = per.entry(c).or_default();
        if let Some(last) = list.last_mut() {
            if last.start == start {
                if pitch > last.note.pitch && c != Channel::Noise {
                    last.note.pitch = pitch;
                }
                v.warnings.push(format!(
                    "{}: a chord at frame {start} keeps its highest note",
                    c.name()
                ));
                continue;
            }
            if last.start + u64::from(last.note.len) > start {
                last.note.len = (start - last.start) as u32;
            }
        }
        let note = Note {
            at: 0,
            len: (stop - start) as u32,
            pitch,
            fine: 0,
            instrument: i,
        };
        let period = note.period(c, region);
        match c {
            Channel::Pulse1 | Channel::Pulse2 | Channel::Triangle
                if period >= 0x7FF || period < if c == Channel::Triangle { 2 } else { 8 } =>
            {
                v.errors.push(format!(
                    "{}: MIDI note {} at frame {start} is out of its range",
                    c.name(),
                    n.key
                ))
            }
            _ => {}
        }
        list.push(Placed { start, note });
    }
    let frames = end.max(1);
    let song = Song {
        name: name.into(),
        kind: Kind::Music,
        origin: Origin::Import,
        confidence: 1.0,
        driver: None,
        frames,
        loop_frame: loop_tick.map(frame_of),
        tempo,
        tracks: per
            .iter()
            .map(|(c, p)| Track::from_placed(*c, p, tempo, frames))
            .collect(),
        request: None,
        event: None,
        level: None,
        state: None,
        captured: None,
        notes: vec!["imported from a MIDI file".into()],
    };
    set.songs.push(song);
    crate::capture::roles(&mut set);
    if let Err(e) = set.check() {
        v.errors.push(e);
    }
    (set, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{self, Options};

    #[test]
    fn export_then_import_keeps_pitches_and_timing() {
        let log = crate::tests::made_up_log(2, None);
        let set = capture::capture(
            Region::Ntsc,
            &log.states(),
            &[],
            &Options {
                name: "made up".into(),
                ..Options::default()
            },
        );
        let s = &set.songs[0];
        let bytes = export(&set, s, &Programs::new());
        assert_eq!(&bytes[..4], b"MThd");
        assert_eq!(bytes[9], 1, "type 1");
        let (back, v) = import(&bytes, "back", Region::Ntsc);
        assert!(v.ok(), "{v:?}");
        let b = &back.songs[0];
        for c in [Channel::Pulse1, Channel::Triangle] {
            let pa: Vec<_> = s.track(c).unwrap().placed(s.tempo);
            let pb: Vec<_> = b.track(c).unwrap().placed(b.tempo);
            assert_eq!(pa.len(), pb.len());
            for (x, y) in pa.iter().zip(&pb) {
                assert_eq!(
                    (x.start, x.note.pitch, x.note.len),
                    (y.start, y.note.pitch, y.note.len)
                );
            }
        }
        assert_eq!(b.loop_frame, s.loop_frame);
        assert_eq!(
            b.track(Channel::Noise).unwrap().placed(b.tempo).len(),
            s.track(Channel::Noise).unwrap().placed(s.tempo).len()
        );
    }

    #[test]
    fn import_refuses_a_fourth_part_and_out_of_range_notes() {
        // Four melodic channels, one note each, and one very low note.
        let mut ev = Vec::new();
        for ch in 0..4u8 {
            ev.push((0u64, 1u8, vec![0x90 | ch, 60, 100]));
            ev.push((48, 0, vec![0x80 | ch, 60, 0]));
        }
        ev.push((48, 1, vec![0x90, 5, 100]));
        ev.push((96, 0, vec![0x80, 5, 0]));
        let mut f = Vec::new();
        chunk(b"MThd", &[0, 1, 0, 1, 0, 96], &mut f);
        chunk(b"MTrk", &track_bytes(ev), &mut f);
        let (_, v) = import(&f, "x", Region::Ntsc);
        let all = v.errors.join("\n");
        assert!(all.contains("fourth melodic part"), "{all}");
        assert!(all.contains("out of its range"), "{all}");
    }
}
