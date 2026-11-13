//! `gp sound render`, `export` and `import` on Tapper's sound (crate::sound_ay):
//! an AY register log or an AY song file played through milestone 3's
//! board sound (gp_arcade::sound, the two AY-8910s, the SSIO's volumes
//! and the speakers, as MAME makes them) into a stereo WAV, and AY songs
//! to and from Standard MIDI files.
//!
//! A register log holds each frame's writes but not their times within
//! the frame, so a render puts every write at its frame's start (Tapper
//! runs at 30 frames a second). The SSIO sets each voice's loudness by a
//! duty cycle read from its PROM; the romset gives it, and without one
//! every duty plays at full volume.
//!
//! MIDI: type 1, one tick a frame (30 ticks a quarter note at 60 beats a
//! minute), a track per AY channel named after it (`0a` to `1c`) on MIDI
//! channels 1 to 6. A note's key is the nearest semitone to its pitch,
//! with a pitch bend (a range of 2 semitones) for the rest, so the period
//! comes back exactly; its velocity and each later frame's volume (CC 11,
//! expression) are the AY volume times 4.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use gp_arcade::attotime::{ATTOSECONDS_PER_SECOND, Attotime};
use gp_arcade::sound::{OUT_RATE, Sound, UPDATE_HZ, to_i16};
use serde_json::{Value, json};

use crate::sound_ay::{AyLog, CLOCK, Write};

/// Tapper's frame rate.
const FPS: i64 = 30;
const NAMES: [&str; 6] = ["0a", "0b", "0c", "1a", "1b", "1c"];

fn at(attos: i128) -> Attotime {
    let s = ATTOSECONDS_PER_SECOND as i128;
    Attotime::new((attos / s) as i32, (attos % s) as i64)
}

/// A register log's frames `log.first` to `log.last` through the board's
/// sound: stereo 16-bit samples at 48 kHz (left AY 0, right AY 1).
pub fn render_log(log: &AyLog, prom: Option<&[u8]>) -> Vec<[i16; 2]> {
    let mut snd = Sound::new(prom.or(Some(&[0u8; 32])));
    if prom.is_none() {
        snd.volumes = [100; 16];
    }
    snd.out = Some(Vec::new());
    let frame = i128::from(ATTOSECONDS_PER_SECOND / FPS);
    let update = i128::from(ATTOSECONDS_PER_SECOND / i64::from(UPDATE_HZ));
    let mut next = update;
    for (k, f) in (log.first..=log.last).enumerate() {
        let t = k as i128 * frame;
        while next <= t {
            snd.update(at(next));
            next += update;
        }
        for w in log.frames.get(&f).into_iter().flatten() {
            snd.write(usize::from(w.chip & 1), 0, w.reg, at(t));
            snd.write(usize::from(w.chip & 1), 2, w.value, at(t));
        }
    }
    let end = (log.last + 1 - log.first) as i128 * frame;
    while next <= end {
        snd.update(at(next));
        next += update;
    }
    snd.out
        .take()
        .unwrap_or_default()
        .into_iter()
        .map(|[l, r]| [to_i16(l), to_i16(r)])
        .collect()
}

/// A 16-bit stereo WAV file.
pub fn wav_stereo(samples: &[[i16; 2]]) -> Vec<u8> {
    let data = (samples.len() * 4) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&OUT_RATE.to_le_bytes());
    out.extend_from_slice(&(OUT_RATE * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for [l, r] in samples {
        out.extend_from_slice(&l.to_le_bytes());
        out.extend_from_slice(&r.to_le_bytes());
    }
    out
}

/// A frame's six channels: each one's (period, volume), or None.
type Frame = [Option<(u16, u8)>; 6];

/// A note open on a track: its key, start tick, period and volumes by
/// tick.
type Open = Option<(u8, u64, u16, Vec<(u64, u8)>)>;

/// Each channel's (period, volume) each frame of a song, frames `first`
/// to `last`; None where it's silent.
fn song_frames(song: &Value) -> (u64, u64, Vec<Frame>) {
    let first = song["first"].as_u64().unwrap_or(0);
    let last = song["last"].as_u64().unwrap_or(first);
    let mut out = vec![[None; 6]; (last + 1 - first) as usize];
    for (c, ch) in song["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .take(6)
    {
        for n in ch["notes"].as_array().into_iter().flatten() {
            let f = n["frame"].as_u64().unwrap_or(0);
            let p = n["period"].as_u64().unwrap_or(0) as u16;
            let vols: Vec<u8> = n["volumes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_u64().map(|v| v as u8))
                .collect();
            for (j, v) in vols.iter().enumerate() {
                if let Some(slot) = (f + j as u64)
                    .checked_sub(first)
                    .and_then(|i| out.get_mut(i as usize))
                {
                    slot[c] = Some((p, *v));
                }
            }
        }
    }
    (first, last, out)
}

/// The register writes that play a song: each frame, the registers of a
/// channel whose state changed (period, mixer, volume), from all off.
/// `loops` more passes go back to the song's `loop_frame` (from its start).
pub fn song_log(song: &Value, loops: u32) -> AyLog {
    let (first, _, frames) = song_frames(song);
    let lf = song["loop_frame"].as_u64().unwrap_or(0) as usize;
    let mut order: Vec<usize> = (0..frames.len()).collect();
    for _ in 0..loops {
        order.extend(lf.min(frames.len())..frames.len());
    }
    let mut log = AyLog {
        first,
        last: first + order.len().max(1) as u64 - 1,
        ..AyLog::default()
    };
    let mut regs = [[0u8; 16]; 2];
    let mut known = [[false; 16]; 2];
    for (k, &i) in order.iter().enumerate() {
        let mut want = regs;
        for (c, st) in frames[i].iter().enumerate() {
            let (chip, ch) = (c / 3, c % 3);
            match st {
                Some((p, v)) => {
                    want[chip][2 * ch] = (*p & 0xff) as u8;
                    want[chip][2 * ch + 1] = (*p >> 8) as u8 & 0x0f;
                    want[chip][8 + ch] = *v & 0x1f;
                    want[chip][7] &= !(1 << ch);
                }
                None => {
                    want[chip][8 + ch] = 0;
                    want[chip][7] |= 1 << ch;
                }
            }
            // Noise off on every channel; the I/O ports as outputs.
            want[chip][7] |= 0x38 | 0xc0;
        }
        let mut ws = Vec::new();
        for chip in 0..2 {
            for reg in 0..11 {
                if want[chip][reg] != regs[chip][reg] || !known[chip][reg] {
                    ws.push(Write {
                        chip: chip as u8,
                        reg: reg as u8,
                        value: want[chip][reg],
                    });
                    known[chip][reg] = true;
                }
            }
        }
        regs = want;
        if !ws.is_empty() {
            log.frames.insert(first + k as u64, ws);
        }
    }
    log
}

fn vlq(mut v: u32, out: &mut Vec<u8>) {
    let mut b = vec![(v & 0x7f) as u8];
    v >>= 7;
    while v > 0 {
        b.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.extend(b.iter().rev());
}

fn chunk(kind: &[u8; 4], body: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
}

fn track(mut ev: Vec<(u64, u32, Vec<u8>)>) -> Vec<u8> {
    ev.sort_by_key(|e| (e.0, e.1));
    let mut body = Vec::new();
    let mut t = 0;
    for (at, _, bytes) in ev {
        vlq((at - t) as u32, &mut body);
        body.extend(bytes);
        t = at;
    }
    body.extend_from_slice(&[0, 0xff, 0x2f, 0]);
    body
}

/// The nearest key and the 14-bit bend for a period.
fn key_of(period: u16) -> (u8, u16) {
    let hz = f64::from(CLOCK) / 16.0 / f64::from(period.max(1));
    let exact = 69.0 + 12.0 * (hz / 440.0).log2();
    let key = exact.round().clamp(0.0, 127.0);
    let bend = (8192.0 + (exact - key) / 2.0 * 8192.0)
        .round()
        .clamp(0.0, 16383.0);
    (key as u8, bend as u16)
}

fn period_of(key: u8, bend: u16) -> u16 {
    let exact = f64::from(key) + (f64::from(bend) - 8192.0) / 8192.0 * 2.0;
    let hz = 440.0 * 2f64.powf((exact - 69.0) / 12.0);
    (f64::from(CLOCK) / 16.0 / hz).round().clamp(1.0, 4095.0) as u16
}

/// A song as a Standard MIDI file.
pub fn midi_export(song: &Value) -> Vec<u8> {
    let first = song["first"].as_u64().unwrap_or(0);
    let mut out = Vec::new();
    let mut head = Vec::new();
    head.extend_from_slice(&1u16.to_be_bytes());
    head.extend_from_slice(&7u16.to_be_bytes());
    head.extend_from_slice(&30u16.to_be_bytes());
    chunk(b"MThd", &head, &mut out);
    let name = song["name"].as_str().unwrap_or("song");
    let mut meta = vec![(0, 0, vec![0xff, 0x51, 3, 0x0f, 0x42, 0x40])];
    let mut nm = vec![0xff, 0x03, name.len().min(127) as u8];
    nm.extend(name.bytes().take(127));
    meta.push((0, 0, nm));
    if let Some(lf) = song["loop_frame"].as_u64() {
        meta.push((lf, 1, vec![0xff, 0x06, 4, b'l', b'o', b'o', b'p']));
    }
    chunk(b"MTrk", &track(meta), &mut out);
    for (c, ch) in song["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .take(6)
    {
        let mc = c as u8;
        let mut ev: Vec<(u64, u32, Vec<u8>)> = Vec::new();
        let label = NAMES[c];
        let mut nm = vec![0xff, 0x03, label.len() as u8];
        nm.extend(label.bytes());
        ev.push((0, 0, nm));
        ev.push((0, 1, vec![0xb0 | mc, 101, 0]));
        ev.push((0, 2, vec![0xb0 | mc, 100, 0]));
        ev.push((0, 3, vec![0xb0 | mc, 6, 2]));
        for n in ch["notes"].as_array().into_iter().flatten() {
            let f = n["frame"].as_u64().unwrap_or(0).saturating_sub(first);
            let len = n["frames"].as_u64().unwrap_or(1).max(1);
            let (key, bend) = key_of(n["period"].as_u64().unwrap_or(1) as u16);
            let vols: Vec<u8> = n["volumes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_u64().map(|v| v as u8))
                .collect();
            let v0 = vols.first().copied().unwrap_or(15);
            ev.push((
                f,
                4,
                vec![0xe0 | mc, (bend & 0x7f) as u8, (bend >> 7) as u8],
            ));
            ev.push((f, 5, vec![0xb0 | mc, 11, (v0 & 0x1f) * 4]));
            ev.push((f, 6, vec![0x90 | mc, key, ((v0 & 0x1f) * 4).max(1)]));
            let mut prev = v0;
            for (j, v) in vols.iter().enumerate().skip(1) {
                if *v != prev {
                    ev.push((f + j as u64, 5, vec![0xb0 | mc, 11, (v & 0x1f) * 4]));
                    prev = *v;
                }
            }
            ev.push((f + len, 3, vec![0x80 | mc, key, 0]));
        }
        chunk(b"MTrk", &track(ev), &mut out);
    }
    out
}

struct Rd<'a> {
    b: &'a [u8],
    i: usize,
}

impl Rd<'_> {
    fn u8(&mut self) -> Result<u8> {
        let v = *self.b.get(self.i).context("the MIDI file ends early")?;
        self.i += 1;
        Ok(v)
    }
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let s = self
            .b
            .get(self.i..self.i + n)
            .context("the MIDI file ends early")?;
        self.i += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into()?))
    }
    fn vlq(&mut self) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..4 {
            let b = self.u8()?;
            v = (v << 7) | u32::from(b & 0x7f);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        bail!("a variable-length number runs past 4 bytes")
    }
}

/// A Standard MIDI file into an AY song file (`gp-ay-songs 0`, one song
/// named `name`), and the warnings and errors found. More than six tracks
/// with notes is an error; a note that starts while another sounds on its
/// track cuts it, with a warning.
pub fn midi_import(bytes: &[u8], name: &str) -> Result<(Value, Vec<String>, Vec<String>)> {
    let mut r = Rd { b: bytes, i: 0 };
    if r.take(4)? != b"MThd" {
        bail!("not a MIDI file (no MThd)");
    }
    let hl = r.u32()? as usize;
    let h = r.take(hl)?.to_vec();
    let division = u16::from_be_bytes([h[4], h[5]]);
    if division & 0x8000 != 0 || division == 0 {
        bail!("SMPTE time division isn't read");
    }
    let (mut warnings, mut errors) = (Vec::new(), Vec::new());
    let mut tempo: Option<u32> = None;
    let mut loop_tick = None;
    // Per track: its name, and its notes (tick, len ticks, period, volumes by tick).
    type Raw = (u64, u64, u16, Vec<(u64, u8)>);
    let mut tracks: Vec<(Option<String>, Vec<Raw>)> = Vec::new();
    while r.i < bytes.len() {
        let kind = r.take(4)?.to_vec();
        let len = r.u32()? as usize;
        let body = r.take(len)?;
        if kind != b"MTrk" {
            continue;
        }
        let mut t = Rd { b: body, i: 0 };
        let mut now = 0u64;
        let mut status = 0u8;
        let mut tname = None;
        let mut notes: Vec<Raw> = Vec::new();
        let mut open: Open = None;
        let mut bend = [8192u16; 16];
        let mut expr = [60u8; 16];
        let close = |open: &mut Open, now: u64, notes: &mut Vec<Raw>| {
            if let Some((_, s, p, v)) = open.take() {
                notes.push((s, now.saturating_sub(s).max(1), p, v));
            }
        };
        while t.i < body.len() {
            now += u64::from(t.vlq()?);
            let mut b = t.u8()?;
            if b == 0xff {
                let mt = t.u8()?;
                let l = t.vlq()? as usize;
                let d = t.take(l)?.to_vec();
                match mt {
                    0x03 => tname = Some(String::from_utf8_lossy(&d).into_owned()),
                    0x06 if d == b"loop" => loop_tick = Some(now),
                    0x51 if d.len() == 3 => {
                        let v = u32::from_be_bytes([0, d[0], d[1], d[2]]);
                        if tempo.is_some_and(|x| x != v) {
                            warnings.push("more than one tempo: the first is used".into());
                        }
                        tempo.get_or_insert(v);
                    }
                    0x2f => break,
                    _ => {}
                }
                continue;
            }
            if b == 0xf0 || b == 0xf7 {
                let l = t.vlq()? as usize;
                t.take(l)?;
                continue;
            }
            if b & 0x80 != 0 {
                status = b;
                b = t.u8()?;
            }
            let ch = usize::from(status & 0x0f);
            let two = !matches!(status & 0xf0, 0xc0 | 0xd0);
            let d2 = if two { t.u8()? } else { 0 };
            match status & 0xf0 {
                0x90 if d2 > 0 => {
                    if open.is_some() {
                        warnings.push(format!(
                            "track {}: a note at tick {now} cuts the one sounding",
                            tracks.len()
                        ));
                    }
                    close(&mut open, now, &mut notes);
                    let p = period_of(b, bend[ch]);
                    let v = (d2 / 4).min(31);
                    expr[ch] = d2;
                    open = Some((b, now, p, vec![(now, v)]));
                }
                0x80 | 0x90 => {
                    if open.as_ref().is_some_and(|o| o.0 == b) {
                        close(&mut open, now, &mut notes);
                    }
                }
                0xb0 if b == 11 => {
                    expr[ch] = d2;
                    if let Some(o) = &mut open {
                        o.3.push((now, (d2 / 4).min(31)));
                    }
                }
                0xe0 => bend[ch] = u16::from(b) | (u16::from(d2) << 7),
                _ => {}
            }
        }
        close(&mut open, now, &mut notes);
        if !notes.is_empty() {
            tracks.push((tname, notes));
        }
    }
    if tracks.len() > 6 {
        errors.push(format!(
            "{} tracks have notes; two AY-8910s have six channels",
            tracks.len()
        ));
    }
    // Ticks to frames: a tick is tempo / division microseconds.
    let us = f64::from(tempo.unwrap_or(500_000)) / f64::from(division);
    let frame = |tick: u64| (tick as f64 * us * FPS as f64 / 1e6).round() as u64;
    let mut slots: [Vec<Value>; 6] = Default::default();
    let mut free: Vec<usize> = (0..6).collect();
    let mut placed = Vec::new();
    for (k, (tn, notes)) in tracks.iter().enumerate().take(6) {
        let c = tn
            .as_deref()
            .and_then(|n| NAMES.iter().position(|x| *x == n))
            .filter(|c| free.contains(c))
            .unwrap_or_else(|| free[0]);
        free.retain(|x| *x != c);
        placed.push((k, c));
        let mut last_end = 0;
        for (s, len, p, vols) in notes {
            let (f, n) = (frame(*s), frame(*s + *len).saturating_sub(frame(*s)).max(1));
            let f = f.max(last_end);
            let mut by: BTreeMap<u64, u8> = BTreeMap::new();
            for (tk, v) in vols {
                by.insert(frame(*tk).max(f), *v);
            }
            let mut cur = by.values().next().copied().unwrap_or(15);
            let volumes: Vec<u8> = (f..f + n)
                .map(|x| {
                    if let Some(v) = by.get(&x) {
                        cur = *v;
                    }
                    cur
                })
                .collect();
            last_end = f + n;
            slots[c].push(json!({
                "frame": f, "frames": n, "period": p,
                "hz": f64::from(CLOCK) / 16.0 / f64::from(*p),
                "volume": volumes[0], "volumes": volumes,
            }));
        }
    }
    let last = slots
        .iter()
        .flatten()
        .map(|n| n["frame"].as_u64().unwrap_or(0) + n["frames"].as_u64().unwrap_or(0))
        .max()
        .unwrap_or(1)
        .saturating_sub(1);
    let channels: Vec<Value> = slots
        .iter()
        .enumerate()
        .map(|(c, notes)| json!({"channel": NAMES[c], "notes": notes}))
        .collect();
    let song = json!({
        "name": name, "origin": "import", "first": 0, "last": last, "frames": last + 1,
        "loop_frame": loop_tick.map(frame), "channels": channels,
    });
    Ok((
        json!({"format": "gp-ay-songs 0", "clock": CLOCK, "first": 0, "last": last, "songs": [song]}),
        warnings,
        errors,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Value {
        json!({"name": "t", "first": 10, "last": 29, "loop_frame": 4, "channels": [
            {"channel": "0a", "notes": [
                {"frame": 10, "frames": 4, "period": 0x1fd, "volume": 15, "volumes": [15, 14, 13, 12]},
                {"frame": 16, "frames": 3, "period": 0x0a0, "volume": 9, "volumes": [9, 9, 9]}]},
            {"channel": "0b", "notes": []},
            {"channel": "0c", "notes": []},
            {"channel": "1a", "notes": [
                {"frame": 20, "frames": 10, "period": 3000, "volume": 16, "volumes": [16, 16, 16, 16, 16, 16, 16, 16, 16, 16]}]},
            {"channel": "1b", "notes": []},
            {"channel": "1c", "notes": []}]})
    }

    #[test]
    fn a_song_plays_back_as_its_own_notes() {
        let s = song();
        let log = song_log(&s, 0);
        let again = crate::sound_ay::decode(
            &log,
            &[crate::sound_ay::SongSpec {
                name: "t".into(),
                first: 10,
                last: 29,
                loop_at: None,
            }],
            60,
        );
        for c in 0..6 {
            let want = s["channels"][c]["notes"].as_array().unwrap();
            let got = again["songs"][0]["channels"][c]["notes"]
                .as_array()
                .unwrap();
            assert_eq!(want.len(), got.len(), "channel {c}");
            for (w, g) in want.iter().zip(got) {
                for k in ["frame", "frames", "period", "volumes"] {
                    assert_eq!(w[k], g[k], "channel {c} {k}");
                }
            }
        }
        assert_eq!(song_log(&s, 2).last, 10 + 20 + 2 * 16 - 1);
        let wav = render_log(&log, None);
        assert!(!wav.is_empty());
        assert!(
            wav.iter().any(|[l, _]| *l != wav[0][0]),
            "the left speaker sounds"
        );
        assert_eq!(wav_stereo(&wav).len(), 44 + wav.len() * 4);
    }

    #[test]
    fn midi_round_trips_the_notes() {
        let s = song();
        let bytes = midi_export(&s);
        let (back, warnings, errors) = midi_import(&bytes, "t").unwrap();
        assert!(
            errors.is_empty() && warnings.is_empty(),
            "{errors:?} {warnings:?}"
        );
        let b = &back["songs"][0];
        assert_eq!(b["loop_frame"], 4);
        for c in [0, 3] {
            let want = s["channels"][c]["notes"].as_array().unwrap();
            let got = b["channels"][c]["notes"].as_array().unwrap();
            assert_eq!(want.len(), got.len(), "channel {c}");
            for (w, g) in want.iter().zip(got) {
                assert_eq!(
                    w["frame"].as_u64().unwrap() - 10,
                    g["frame"].as_u64().unwrap()
                );
                for k in ["frames", "period", "volumes"] {
                    assert_eq!(w[k], g[k], "{k}");
                }
            }
        }
    }
}
