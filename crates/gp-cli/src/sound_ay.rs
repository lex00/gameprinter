//! `gp sound capture`, `fidelity` and `compare` on an arcade recording
//! (port wave 6): the AY-8910 register log of Tapper's sound board, from
//! the recording's `ay-write` events (or a replay with the event log on),
//! in place of the NES's APU log.
//!
//! The log, `ay.log`, is text: a header (`gp-ay-log 0`, the chips and their
//! clock), then one line per frame that wrote anything, the frame and its
//! writes in order, `chip.reg=value` in hex:
//!
//! ```text
//! gp-ay-log 0
//! chips 2 clock 2000000
//! frames 700 760
//! 701 0.07=38 0.08=0f 1.00=fd 1.01=01
//! ```
//!
//! A channel's state at the end of a frame is what its registers say: the
//! tone period (registers 0 to 5, 12 bits), whether the mixer (register 7)
//! lets its tone and noise through, and its volume (registers 8 to 10, bit
//! 4 the envelope). The capture's `songs.json` holds each of the six
//! channels' notes: runs of frames with the same period while the tone is
//! on and the volume isn't 0, with the pitch in hertz (clock / 16 /
//! period) and the volume each frame of it. `fidelity` holds the notes to the log, frame by frame: the
//! onsets, the pitch and the volume each frame they give against the log's
//! channel states. `compare` holds one log to another: the frames whose
//! writes are the same, and those whose every channel's state is.
//!
//! What the NES's capture also does and this doesn't: songs bound to a
//! model's levels and the game's sound requests (`--template`), and the
//! renders' loudness. `gp sound render` plays a log or
//! its notes through milestone 3's AY synth (crate::sound_ay_render).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use gp_evidence::events::EventKind;
use serde_json::{Value, json};

/// The AY-8910s' clock on the SSIO board: its 16 MHz crystal over 8.
pub const CLOCK: u32 = 2_000_000;
pub const HEADER: &str = "gp-ay-log 0";

/// One register write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Write {
    pub chip: u8,
    pub reg: u8,
    pub value: u8,
}

/// A register log: each frame's writes, by frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AyLog {
    pub first: u64,
    pub last: u64,
    pub frames: BTreeMap<u64, Vec<Write>>,
}

impl AyLog {
    pub fn text(&self) -> String {
        let mut s = format!(
            "{HEADER}\nchips 2 clock {CLOCK}\nframes {} {}\n",
            self.first, self.last
        );
        for (f, ws) in &self.frames {
            let _ = write!(s, "{f}");
            for w in ws {
                let _ = write!(s, " {}.{:02x}={:02x}", w.chip, w.reg, w.value);
            }
            s.push('\n');
        }
        s
    }

    pub fn parse(text: &str) -> Result<Self> {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            bail!("not an AY register log (no `{HEADER}` line)");
        }
        let mut log = Self::default();
        for (n, line) in lines.enumerate() {
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("chips") | None => continue,
                Some("frames") => {
                    let mut num = || -> Result<u64> {
                        Ok(parts.next().context("frames FIRST LAST")?.parse()?)
                    };
                    log.first = num()?;
                    log.last = num()?;
                }
                Some(f) => {
                    let frame: u64 = f
                        .parse()
                        .with_context(|| format!("line {}: {f:?} isn't a frame", n + 2))?;
                    let mut ws = Vec::new();
                    for p in parts {
                        let bad = || anyhow::anyhow!("line {}: {p:?} isn't chip.reg=value", n + 2);
                        let (cr, v) = p.split_once('=').ok_or_else(bad)?;
                        let (c, r) = cr.split_once('.').ok_or_else(bad)?;
                        ws.push(Write {
                            chip: c.parse().map_err(|_| bad())?,
                            reg: u8::from_str_radix(r, 16).map_err(|_| bad())?,
                            value: u8::from_str_radix(v, 16).map_err(|_| bad())?,
                        });
                    }
                    log.frames.insert(frame, ws);
                }
            }
        }
        Ok(log)
    }

    /// From a recording's events (any kinds; the `ay-write`s are taken).
    pub fn of_events(events: &[gp_evidence::events::Event], first: u64, last: u64) -> Self {
        let mut log = Self {
            first,
            last,
            ..Self::default()
        };
        for e in events.iter().filter(|e| e.kind == EventKind::AyWrite) {
            log.frames.entry(e.frame).or_default().push(Write {
                chip: (e.addr >> 8) as u8,
                reg: (e.addr & 0x0f) as u8,
                value: e.value,
            });
        }
        log
    }

    /// Each frame's six channels' state at its end, frames `first` to
    /// `last`, from the registers as the log leaves them (all 0 before).
    pub fn states(&self) -> Vec<(u64, [Channel; 6])> {
        let mut regs = [[0u8; 16]; 2];
        let mut out = Vec::new();
        for f in self.first..=self.last {
            for w in self.frames.get(&f).into_iter().flatten() {
                regs[usize::from(w.chip & 1)][usize::from(w.reg & 0x0f)] = w.value;
            }
            let mut chans = [Channel::default(); 6];
            for (i, ch) in chans.iter_mut().enumerate() {
                let r = &regs[i / 3];
                let c = i % 3;
                *ch = Channel {
                    period: (u16::from(r[2 * c + 1] & 0x0f) << 8) | u16::from(r[2 * c]),
                    tone: r[7] & (1 << c) == 0,
                    noise: r[7] & (8 << c) == 0,
                    volume: r[8 + c] & 0x1f,
                };
            }
            out.push((f, chans));
        }
        out
    }
}

/// One channel's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Channel {
    pub period: u16,
    pub tone: bool,
    pub noise: bool,
    /// 0 to 15, or 16 and over when the envelope sets it.
    pub volume: u8,
}

impl Channel {
    fn sounds(self) -> bool {
        self.tone && self.volume != 0 && self.period != 0
    }
}

const NAMES: [&str; 6] = ["0a", "0b", "0c", "1a", "1b", "1c"];

/// The notes: each channel's runs of a sounding tone at one period.
pub fn notes(log: &AyLog) -> Value {
    let channels = channel_notes(&log.states());
    json!({"format": "gp-ay-songs 0", "clock": CLOCK, "first": log.first, "last": log.last, "channels": channels})
}

/// Each of the six channels' notes over `states`, as `notes` gives them.
fn channel_notes(states: &[(u64, [Channel; 6])]) -> Vec<Value> {
    let mut channels = Vec::new();
    for (c, name) in NAMES.iter().enumerate() {
        let mut notes: Vec<Value> = Vec::new();
        let mut cur: Option<(u64, u16, Vec<u8>)> = None;
        let close = |cur: &mut Option<(u64, u16, Vec<u8>)>, notes: &mut Vec<Value>| {
            if let Some((start, period, volumes)) = cur.take() {
                notes.push(json!({
                    "frame": start, "frames": volumes.len(), "period": period,
                    "hz": f64::from(CLOCK) / 16.0 / f64::from(period),
                    "volume": volumes[0], "volumes": volumes,
                }));
            }
        };
        for (f, chans) in states {
            let ch = chans[c];
            match cur {
                Some((_, p, ref mut v)) if ch.sounds() && ch.period == p => v.push(ch.volume),
                _ => {
                    close(&mut cur, &mut notes);
                    if ch.sounds() {
                        cur = Some((*f, ch.period, vec![ch.volume]));
                    }
                }
            }
        }
        close(&mut cur, &mut notes);
        channels.push(json!({"channel": name, "notes": notes}));
    }
    channels
}

/// A song asked for by name: frames `first` to `last` of the log, and the
/// frame its loop goes back to, counted from `first`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongSpec {
    pub name: String,
    pub first: u64,
    pub last: u64,
    pub loop_at: Option<u64>,
}

/// The midway-ssio decoder (gp_sound::drivers): the songs a register log
/// holds. Given `specs`, those frame ranges; else each run of frames where
/// a channel sounds, ended by `gap` frames of silence on all six. Each
/// song holds its channels' notes as `notes` gives them, with the
/// registers as the log leaves them before its first frame.
pub fn decode(log: &AyLog, specs: &[SongSpec], gap: u64) -> Value {
    let states = log.states();
    let sounding = |chans: &[Channel; 6]| chans.iter().any(|c| c.sounds());
    let mut ranges: Vec<SongSpec> = specs.to_vec();
    if ranges.is_empty() {
        let mut cur: Option<(u64, u64)> = None;
        let mut k = 0;
        let mut push = |cur: (u64, u64), ranges: &mut Vec<SongSpec>| {
            ranges.push(SongSpec {
                name: format!("song{k}"),
                first: cur.0,
                last: cur.1,
                loop_at: None,
            });
            k += 1;
        };
        for (f, chans) in &states {
            if !sounding(chans) {
                continue;
            }
            cur = match cur {
                Some((a, b)) if f - b <= gap => Some((a, *f)),
                Some(done) => {
                    push(done, &mut ranges);
                    Some((*f, *f))
                }
                None => Some((*f, *f)),
            };
        }
        if let Some(done) = cur {
            push(done, &mut ranges);
        }
    }
    let songs: Vec<Value> = ranges
        .iter()
        .map(|s| {
            let part: Vec<(u64, [Channel; 6])> = states
                .iter()
                .filter(|(f, _)| (s.first..=s.last).contains(f))
                .copied()
                .collect();
            json!({
                "name": s.name, "origin": "driver", "driver": "midway-ssio",
                "first": s.first, "last": s.last, "frames": s.last + 1 - s.first,
                "loop_frame": s.loop_at, "channels": channel_notes(&part),
            })
        })
        .collect();
    json!({"format": "gp-ay-songs 0", "clock": CLOCK, "first": log.first, "last": log.last, "songs": songs})
}

/// The songs of an AY song file: a decode's `songs`, or a capture's
/// notes as one song named `name`.
pub fn songs_of(v: &Value, name: &str) -> Vec<Value> {
    if let Some(s) = v["songs"].as_array() {
        return s.clone();
    }
    let mut one = v.clone();
    one["name"] = json!(name);
    vec![one]
}

/// Each note's start: (channel index, frame, period).
fn onsets(song: &Value) -> Vec<(usize, u64, u64)> {
    let mut out = Vec::new();
    for (c, ch) in song["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        for n in ch["notes"].as_array().into_iter().flatten() {
            out.push((
                c,
                n["frame"].as_u64().unwrap_or(0),
                n["period"].as_u64().unwrap_or(0),
            ));
        }
    }
    out
}

/// A decoded song held to a captured one: their note starts (channel,
/// frame, period) matched at the offset that matches most, as an F1.
/// The offset is the captured frame less the decoded one.
pub fn cross_check(decoded: &Value, captured: &Value) -> Value {
    let dec = onsets(decoded);
    let cap = onsets(captured);
    let set: std::collections::BTreeSet<(usize, u64, u64)> = cap.iter().copied().collect();
    let mut offsets: Vec<i64> = vec![0];
    if let Some(&(c0, f0, p0)) = dec.first() {
        offsets.extend(
            cap.iter()
                .filter(|&&(c, _, p)| c == c0 && p == p0)
                .map(|&(_, f, _)| f as i64 - f0 as i64)
                .take(4000),
        );
    }
    offsets.sort_unstable();
    offsets.dedup();
    let (mut best_off, mut best) = (0i64, 0usize);
    for off in offsets {
        let m = dec
            .iter()
            .filter(|&&(c, f, p)| {
                u64::try_from(f as i64 + off).is_ok_and(|g| set.contains(&(c, g, p)))
            })
            .count();
        if m > best {
            (best_off, best) = (off, m);
        }
    }
    let f1 = if dec.is_empty() || cap.is_empty() {
        0.0
    } else {
        let (p, r) = (
            best as f64 / dec.len() as f64,
            best as f64 / cap.len() as f64,
        );
        if p + r == 0.0 {
            0.0
        } else {
            (2.0 * p * r / (p + r) * 1000.0).round() / 1000.0
        }
    };
    json!({"offset": best_off, "matched": best, "decoded": dec.len(), "reference": cap.len(), "f1": f1})
}

/// The channel states the notes give each frame: the period and volume of
/// the note sounding, or silence.
fn states_of_notes(songs: &Value, first: u64, last: u64) -> Vec<[Option<(u16, u8)>; 6]> {
    let mut out = vec![[None; 6]; (last - first + 1) as usize];
    for (c, ch) in songs["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .take(6)
    {
        for n in ch["notes"].as_array().into_iter().flatten() {
            let (f, len) = (
                n["frame"].as_u64().unwrap_or(0),
                n["frames"].as_u64().unwrap_or(0),
            );
            let p = n["period"].as_u64().unwrap_or(0) as u16;
            let v0 = n["volume"].as_u64().unwrap_or(0) as u8;
            let vols = n["volumes"].as_array();
            for (j, k) in (f..f + len).enumerate() {
                let v = vols
                    .and_then(|a| a.get(j))
                    .and_then(Value::as_u64)
                    .map_or(v0, |v| v as u8);
                if let Some(slot) = k.checked_sub(first).and_then(|i| out.get_mut(i as usize)) {
                    slot[c] = Some((p, v));
                }
            }
        }
    }
    out
}

/// `gp sound fidelity` on an AY capture: the notes against the log.
pub fn fidelity(dir: &Path) -> Result<(String, Value)> {
    let log = AyLog::parse(&std::fs::read_to_string(dir.join("ay.log"))?)?;
    let songs: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("songs.json"))?)?;
    let want = log.states();
    let have = states_of_notes(&songs, log.first, log.last);
    let (mut onsets, mut onsets_ok, mut frames, mut pitch_ok, mut volume_ok) = (0, 0, 0, 0, 0);
    let mut prev = [Channel::default(); 6];
    for ((_, chans), got) in want.iter().zip(&have) {
        for c in 0..6 {
            let w = chans[c];
            let sounding = w.sounds();
            if sounding && (!prev[c].sounds() || prev[c].period != w.period) {
                onsets += 1;
                onsets_ok += u32::from(got[c].is_some_and(|(p, _)| p == w.period));
            }
            if sounding {
                frames += 1;
                pitch_ok += u32::from(got[c].is_some_and(|(p, _)| p == w.period));
                volume_ok += u32::from(got[c].is_some_and(|(_, v)| v == w.volume));
            }
        }
        prev = *chans;
    }
    let pct = |a: u32, b: u32| {
        if b == 0 {
            100.0
        } else {
            f64::from(a) * 100.0 / f64::from(b)
        }
    };
    let text = format!(
        "AY notes against their log, frames {} to {}: onsets {onsets_ok} of {onsets} ({:.1}%), pitch {pitch_ok} of {frames} sounding channel frames ({:.1}%), volume {volume_ok} of {frames} ({:.1}%)",
        log.first,
        log.last,
        pct(onsets_ok, onsets),
        pct(pitch_ok, frames),
        pct(volume_ok, frames)
    );
    Ok((
        text,
        json!({"onsets": onsets, "onsets_matched": onsets_ok, "frames": frames, "pitch_matched": pitch_ok, "volume_matched": volume_ok}),
    ))
}

/// `gp sound compare` of two AY logs.
pub fn compare(a: &AyLog, b: &AyLog, offset: i64) -> (String, Value, u8) {
    let sb: BTreeMap<u64, [Channel; 6]> = b.states().into_iter().collect();
    let (mut same_writes, mut same_state, mut n) = (0u64, 0u64, 0u64);
    let mut first_bad = None;
    for (f, chans) in a.states() {
        let Some(g) = u64::try_from(f as i64 - offset).ok() else {
            continue;
        };
        let Some(other) = sb.get(&g) else { continue };
        n += 1;
        same_writes += u64::from(a.frames.get(&f) == b.frames.get(&g));
        if chans == *other {
            same_state += 1;
        } else if first_bad.is_none() {
            first_bad = Some(f);
        }
    }
    let text = format!(
        "{n} frames compared: writes the same on {same_writes}, every channel's state the same on {same_state}{}",
        first_bad.map_or_else(String::new, |f| format!("; first differing frame {f}"))
    );
    (
        text,
        json!({"frames": n, "same_writes": same_writes, "same_state": same_state, "first_differing": first_bad}),
        u8::from(first_bad.is_some()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> AyLog {
        let w = |chip, reg, value| Write { chip, reg, value };
        let mut l = AyLog {
            first: 1,
            last: 6,
            ..AyLog::default()
        };
        // Chip 0 channel A: period 0x1fd, tone on, volume 15, from frame 2
        // to 4; then a new period on frame 5.
        l.frames.insert(
            2,
            vec![w(0, 0, 0xfd), w(0, 1, 0x01), w(0, 7, 0x3e), w(0, 8, 0x0f)],
        );
        l.frames.insert(5, vec![w(0, 0, 0x80)]);
        l.frames.insert(6, vec![w(0, 8, 0x00)]);
        l
    }

    #[test]
    fn a_log_reads_back_and_gives_notes() {
        let l = log();
        assert_eq!(AyLog::parse(&l.text()).unwrap(), l);
        let s = notes(&l);
        let a = &s["channels"][0]["notes"];
        assert_eq!(a.as_array().unwrap().len(), 2);
        assert_eq!(
            (
                a[0]["frame"].as_u64(),
                a[0]["frames"].as_u64(),
                a[0]["period"].as_u64()
            ),
            (Some(2), Some(3), Some(0x1fd))
        );
        assert_eq!(a[1]["period"], 0x180);
        assert!(s["channels"][1]["notes"].as_array().unwrap().is_empty());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ay.log"), l.text()).unwrap();
        std::fs::write(dir.path().join("songs.json"), s.to_string()).unwrap();
        let (text, v) = fidelity(dir.path()).unwrap();
        assert_eq!(v["onsets"], v["onsets_matched"], "{text}");
        assert_eq!(v["frames"], v["pitch_matched"]);
        let (_, c, exit) = compare(&l, &l, 0);
        assert_eq!((c["same_state"].as_u64(), exit), (Some(6), 0));
        let mut other = l.clone();
        other.frames.insert(
            3,
            vec![Write {
                chip: 0,
                reg: 8,
                value: 3,
            }],
        );
        let (_, c, exit) = compare(&l, &other, 0);
        assert_eq!((c["first_differing"].as_u64(), exit), (Some(3), 1));
    }

    #[test]
    fn the_decoder_splits_songs_at_silences_and_cross_checks() {
        let mut l = log();
        l.last = 200;
        let w = |chip, reg, value| Write { chip, reg, value };
        // A second run of sound on chip 1, channel A, 100 frames later.
        l.frames
            .insert(100, vec![w(1, 0, 0x40), w(1, 7, 0x3e), w(1, 8, 0x0c)]);
        l.frames.insert(110, vec![w(1, 8, 0x00)]);
        let d = decode(&l, &[], 60);
        let songs = d["songs"].as_array().unwrap();
        assert_eq!(songs.len(), 2);
        assert_eq!(
            (songs[0]["first"].as_u64(), songs[0]["last"].as_u64()),
            (Some(2), Some(5))
        );
        assert_eq!(songs[1]["channels"][3]["notes"][0]["frame"], 100);
        let named = decode(
            &l,
            &[SongSpec {
                name: "all".into(),
                first: 1,
                last: 200,
                loop_at: None,
            }],
            60,
        );
        assert_eq!(named["songs"][0]["name"], "all");
        let cap = notes(&l);
        let c = cross_check(&named["songs"][0], &songs_of(&cap, "cap")[0]);
        assert_eq!(
            (c["f1"].as_f64(), c["offset"].as_i64()),
            (Some(1.0), Some(0))
        );
        let c = cross_check(&songs[1], &cap);
        assert_eq!(c["matched"], 1);
    }

    /// With tapper.zip on this machine: the log made from the event
    /// stream holds the same register writes, frame by frame, as
    /// milestone 3's AY write list (Tapper::ay_writes, held to MAME's).
    /// Skipped without it.
    #[test]
    fn the_events_give_milestone_3s_writes_on_tapper() {
        let Some(zip) = gp_arcade::romset::local_romset("tapper") else {
            eprintln!("skipped: no tapper romset");
            return;
        };
        let set = gp_arcade::Romset::load_zip(&zip).unwrap();
        let mut t = gp_arcade::Tapper::power_on(&set);
        t.set_events(true);
        let mut latch = [None::<u8>; 2];
        let (mut checked, mut writes) = (0, 0);
        for f in 1..=900u64 {
            let coin = (600..606).contains(&f);
            t.step(gp_arcade::Controls {
                coin,
                ..gp_arcade::Controls::default()
            });
            let events: Vec<_> = t
                .take_events()
                .iter()
                .map(|e| crate::arcade_rec::stored(f, e))
                .collect();
            let log = AyLog::of_events(&events, f, f);
            let mut want = Vec::new();
            for &(chip, l, v) in t.ay_writes() {
                let c = usize::from(chip);
                if l == 0 {
                    latch[c] = (v >> 4 == 0).then_some(v & 0x0f);
                } else if let Some(reg) = latch[c] {
                    want.push(Write {
                        chip,
                        reg,
                        value: v,
                    });
                }
            }
            writes += want.len();
            assert_eq!(
                log.frames.get(&f).cloned().unwrap_or_default(),
                want,
                "frame {f}"
            );
            checked += 1;
        }
        assert_eq!(checked, 900);
        assert!(writes > 100, "{writes}");
    }
}
