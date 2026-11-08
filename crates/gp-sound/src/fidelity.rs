//! The fidelity check (docs/sound.md, "Fidelity"): a captured song played
//! by the player against the register log it came from, both through the
//! same APU model and synthesizer. Notes are compared by onset (same
//! channel, same pitch, within a frame), frames by period and volume, and
//! the two renders by their loudness curves.

use serde::Serialize;

use crate::apu::{Channel, ChannelState, Region, pitch};
use crate::capture::{RawNote, notes};

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Agreement {
    /// Onsets in the reference and in the candidate, and those matched.
    pub reference_notes: usize,
    pub candidate_notes: usize,
    pub matched: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    /// Of the frames a channel sounds in the reference, the share where
    /// the candidate has the same period, and the same volume within 1.
    pub pitch_frames: f64,
    pub volume_frames: f64,
    /// The Pearson correlation of the two renders' loudness per frame.
    pub loudness: Option<f64>,
    /// Per channel: reference notes, matched.
    pub channels: Vec<(String, usize, usize)>,
}

fn same_pitch(c: Channel, a: &RawNote, b: &RawNote, region: Region) -> bool {
    match c {
        Channel::Noise | Channel::Dmc => a.period == b.period,
        _ => match (pitch(c, a.period, region), pitch(c, b.period, region)) {
            (Some(x), Some(y)) => (x - y).abs() < 0.5,
            _ => a.period == b.period,
        },
    }
}

/// Compares candidate states to reference states. `mask[c][f]` leaves a
/// reference frame out (an effect's, when checking music).
///
/// The candidate's notes are `intended` when given (a song's own notes, so
/// a player's register habits don't split them), else derived from its
/// states like the reference's.
pub fn compare(
    region: Region,
    reference: &[[ChannelState; 5]],
    candidate: &[[ChannelState; 5]],
    mask: Option<&[Vec<bool>; 5]>,
    intended: Option<Vec<RawNote>>,
) -> Agreement {
    let n = reference.len().min(candidate.len());
    let reference = &reference[..n];
    let candidate = &candidate[..n];
    let ra = notes(reference, mask);
    let masked = |c: Channel, f: u64| {
        mask.is_some_and(|m| m[c.index()].get(f as usize).copied().unwrap_or(false))
    };
    let rb = match intended {
        Some(v) => v
            .into_iter()
            .filter(|x| (x.start as usize) < n && !masked(x.channel, x.start))
            .collect(),
        None => notes(candidate, mask),
    };
    let mut a = Agreement {
        reference_notes: ra.len(),
        candidate_notes: rb.len(),
        ..Agreement::default()
    };
    let mut used = vec![false; rb.len()];
    for c in Channel::ALL {
        let mut refs = 0;
        let mut hit = 0;
        for x in ra.iter().filter(|x| x.channel == c) {
            refs += 1;
            let m = rb.iter().enumerate().find(|(i, y)| {
                !used[*i]
                    && y.channel == c
                    && y.start.abs_diff(x.start) <= 1
                    && same_pitch(c, x, y, region)
            });
            if let Some((i, _)) = m {
                used[i] = true;
                hit += 1;
            }
        }
        if refs > 0 {
            a.channels.push((c.name().into(), refs, hit));
        }
        a.matched += hit;
    }
    a.precision = ratio(a.matched, a.candidate_notes);
    a.recall = ratio(a.matched, a.reference_notes);
    a.f1 = if a.precision + a.recall > 0.0 {
        r3(2.0 * a.precision * a.recall / (a.precision + a.recall))
    } else {
        0.0
    };
    let mut sounding = 0;
    let mut p_ok = 0;
    let mut v_ok = 0;
    for f in 0..n {
        for c in 0..5 {
            if mask.is_some_and(|m| m[c].get(f).copied().unwrap_or(false)) {
                continue;
            }
            let (x, y) = (reference[f][c], candidate[f][c]);
            if x.volume == 0 {
                continue;
            }
            sounding += 1;
            if y.volume > 0 && x.period == y.period {
                p_ok += 1;
            }
            if x.volume.abs_diff(y.volume) <= 1 {
                v_ok += 1;
            }
        }
    }
    a.pitch_frames = ratio(p_ok, sounding);
    a.volume_frames = ratio(v_ok, sounding);
    a
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 { 1.0 } else { r3(a as f64 / b as f64) }
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The Pearson correlation of two curves, over their common length.
pub fn correlation(a: &[f64], b: &[f64]) -> Option<f64> {
    let n = a.len().min(b.len());
    if n < 2 {
        return None;
    }
    let ma = a[..n].iter().sum::<f64>() / n as f64;
    let mb = b[..n].iter().sum::<f64>() / n as f64;
    let mut sab = 0.0;
    let mut saa = 0.0;
    let mut sbb = 0.0;
    for i in 0..n {
        let (x, y) = (a[i] - ma, b[i] - mb);
        sab += x * y;
        saa += x * x;
        sbb += y * y;
    }
    if saa == 0.0 || sbb == 0.0 {
        return None;
    }
    Some(r3(sab / (saa * sbb).sqrt()))
}

/// A decoded song against a captured one, by note onsets, at the time
/// offset into the decoded song where they agree most.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Onsets {
    /// The decoded song's frame the captured song's frame 0 lines up with.
    pub offset: i64,
    pub reference: usize,
    pub candidate: usize,
    pub matched: usize,
    pub f1: f64,
    /// Per channel: captured notes, matched.
    pub channels: Vec<(String, usize, usize)>,
}

/// Cross-checks a decoded song against a captured one.
pub fn cross_check(decoded: &crate::song::Song, captured: &crate::song::Song) -> Onsets {
    let cap: Vec<(Channel, u64, u8)> = captured
        .tracks
        .iter()
        .flat_map(|t| {
            t.placed(captured.tempo)
                .into_iter()
                .map(move |p| (t.channel, p.start, p.note.pitch))
        })
        .collect();
    let horizon = captured.frames + 4 * decoded.frames.max(1);
    let dec = unrolled(decoded, horizon);
    let mut best = Onsets::default();
    let Some(&(c0, s0, p0)) = cap
        .iter()
        .filter(|x| x.0 != Channel::Noise)
        .min_by_key(|x| x.1)
    else {
        return best;
    };
    let mut tried = std::collections::BTreeSet::new();
    for &(c, s, p) in &dec {
        if c != c0 || p != p0 || s < s0 {
            continue;
        }
        let off = (s - s0) as i64;
        if !tried.insert(off) || off as u64 > decoded.frames {
            continue;
        }
        let o = score(&cap, &dec, off, captured.frames);
        if o.matched > best.matched {
            best = o;
        }
    }
    best
}

fn score(cap: &[(Channel, u64, u8)], dec: &[(Channel, u64, u8)], off: i64, frames: u64) -> Onsets {
    let window: Vec<&(Channel, u64, u8)> = dec
        .iter()
        .filter(|d| d.1 as i64 >= off && ((d.1 as i64 - off) as u64) < frames)
        .collect();
    let mut used = vec![false; window.len()];
    let mut per: std::collections::BTreeMap<Channel, (usize, usize)> = Default::default();
    let mut matched = 0;
    for &(c, s, p) in cap {
        let e = per.entry(c).or_default();
        e.0 += 1;
        let t = s as i64 + off;
        if let Some(i) = window.iter().enumerate().position(|(i, d)| {
            !used[i] && d.0 == c && (d.1 as i64 - t).abs() <= 1 && (c == Channel::Noise || d.2 == p)
        }) {
            used[i] = true;
            e.1 += 1;
            matched += 1;
        }
    }
    let f1 = if cap.len() + window.len() == 0 {
        1.0
    } else {
        r3(2.0 * matched as f64 / (cap.len() + window.len()) as f64)
    };
    Onsets {
        offset: off,
        reference: cap.len(),
        candidate: window.len(),
        matched,
        f1,
        channels: per
            .into_iter()
            .map(|(c, (a, b))| (c.name().to_owned(), a, b))
            .collect(),
    }
}

/// A song's onsets with its loop played until `horizon`.
fn unrolled(s: &crate::song::Song, horizon: u64) -> Vec<(Channel, u64, u8)> {
    let lf = s.loop_frame.unwrap_or(s.frames);
    let body = s.frames.saturating_sub(lf);
    let mut out = Vec::new();
    for t in &s.tracks {
        let placed = t.placed(s.tempo);
        for p in &placed {
            out.push((t.channel, p.start, p.note.pitch));
        }
        if body == 0 {
            continue;
        }
        let mut pass = 1u64;
        while s.frames + (pass - 1) * body < horizon {
            for p in placed.iter().filter(|p| p.start >= lf) {
                out.push((t.channel, p.start + pass * body, p.note.pitch));
            }
            pass += 1;
        }
    }
    out.sort_by_key(|x| x.1);
    out
}
