//! `gp sound render` on Tapper's sound (crate::sound_ay): an AY register
//! log or an AY song file played through milestone 3's board sound
//! (gp_arcade::sound, the two AY-8910s, the SSIO's volumes and the
//! speakers, as MAME makes them) into a stereo WAV.
//!
//! A register log holds each frame's writes but not their times within
//! the frame, so a render puts every write at its frame's start (Tapper
//! runs at 30 frames a second). The SSIO sets each voice's loudness by a
//! duty cycle read from its PROM; the romset gives it, and without one
//! every duty plays at full volume.

use gp_arcade::attotime::{ATTOSECONDS_PER_SECOND, Attotime};
use gp_arcade::sound::{OUT_RATE, Sound, UPDATE_HZ, to_i16};
use serde_json::Value;

use crate::sound_ay::{AyLog, Write};

/// Tapper's frame rate.
const FPS: i64 = 30;

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

#[cfg(test)]
mod tests {
    use serde_json::json;

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
}
