//! From a register log to notes and instruments, for any NES game
//! (docs/sound.md, "Capture"). Nothing here knows a game or a driver.
//!
//! 1. The log is applied to the APU model and each channel's state read at
//!    the end of every frame: timer period, volume, duty, and whether its
//!    fourth register was written (a trigger).
//! 2. Notes: a channel starts a note when it begins to sound, or when it's
//!    triggered and its period changed or its volume rose (the envelope
//!    restarted). Changes without a trigger stay in the note as its
//!    envelopes: volume, duty and period offset per frame, which is how
//!    vibrato, slides and sweeps are kept.
//! 3. Instruments: notes on the same kind of channel whose envelopes agree
//!    over the frames they share (volume within 1 on 90% of them, duty and
//!    pitch offsets equal) are one instrument, whose envelopes are its
//!    longest note's.
//! 4. Effects: each sound request (gp gaps) claims the channels that start
//!    a note within 3 frames of it, until the channel falls silent, the
//!    next request on it, or 120 frames. A channel named in the request's
//!    interface (square1, pulse 2, noise, triangle, dpcm) is the one it
//!    claims. Music is what's left.
//! 5. Tempo: the largest frame count up to 32 that divides 90% of the
//!    note starts' distances from the first. Loop: the smallest period
//!    after which every unclaimed channel's state repeats to the end of the
//!    capture, for at least one full period.

use std::collections::BTreeMap;

use crate::apu::{Channel, ChannelState, Region, pitch};
use crate::song::{
    Captured, Instrument, Kind, Note, Origin, Placed, Request, Song, SongSet, Tempo, Track, Voice,
};

/// Frames after a request a channel must start a note in to be claimed.
pub const CLAIM_WITHIN: u64 = 3;
/// The longest an effect is taken to last.
pub const MAX_EFFECT: u64 = 120;
/// The shortest loop period looked for, in frames.
pub const MIN_LOOP: usize = 60;

/// A note as captured, with its envelopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNote {
    pub channel: Channel,
    pub start: u64,
    pub period: u16,
    pub volume: Vec<u8>,
    pub duty: Vec<u8>,
    pub pitch: Vec<i16>,
    pub sample: Option<(u16, u16)>,
}

impl RawNote {
    pub fn len(&self) -> u64 {
        self.volume.len() as u64
    }
    pub fn is_empty(&self) -> bool {
        self.volume.is_empty()
    }
}

/// Notes per channel from frame states. `mask[c][f]` true leaves frame `f`
/// of channel `c` out (another sound has it).
pub fn notes(states: &[[ChannelState; 5]], mask: Option<&[Vec<bool>; 5]>) -> Vec<RawNote> {
    let mut out = Vec::new();
    for c in Channel::ALL {
        let ci = c.index();
        let mut cur: Option<RawNote> = None;
        let mut last_vol = 0u8;
        for (f, st) in states.iter().enumerate() {
            let s = st[ci];
            let masked = mask.is_some_and(|m| m[ci].get(f).copied().unwrap_or(false));
            if s.volume == 0 || masked {
                if let Some(n) = cur.take() {
                    out.push(n);
                }
                last_vol = 0;
                continue;
            }
            let new = match &cur {
                None => true,
                Some(n) => {
                    let last_period =
                        (i32::from(n.period) + i32::from(*n.pitch.last().unwrap_or(&0))) as u16;
                    // A pulse retrigger that moves the pitch less than a
                    // quarter tone with no envelope restart is a driver
                    // writing the high period byte mid-note.
                    let moved = |a: u16, b: u16| match (
                        pitch(c, a, Region::Ntsc),
                        pitch(c, b, Region::Ntsc),
                    ) {
                        (Some(x), Some(y)) => (x - y).abs() >= 0.5,
                        _ => a != b,
                    };
                    match c {
                        Channel::Noise | Channel::Dmc | Channel::Triangle => s.trigger,
                        _ => s.trigger && (moved(s.period, last_period) || s.volume > last_vol),
                    }
                }
            };
            if new {
                if let Some(n) = cur.take() {
                    out.push(n);
                }
                cur = Some(RawNote {
                    channel: c,
                    start: f as u64,
                    period: s.period,
                    volume: Vec::new(),
                    duty: Vec::new(),
                    pitch: Vec::new(),
                    sample: s.sample,
                });
            }
            let n = cur.as_mut().expect("a note is open");
            n.volume.push(s.volume);
            n.duty.push(s.duty);
            n.pitch
                .push((i32::from(s.period) - i32::from(n.period)) as i16);
            last_vol = s.volume;
        }
        if let Some(n) = cur.take() {
            out.push(n);
        }
    }
    out.sort_by_key(|n| (n.start, n.channel));
    out
}

/// Whether a note fits an instrument's envelopes over the frames they
/// share.
fn fits(ins: &Instrument, n: &RawNote) -> bool {
    if ins.voice != Voice::of(n.channel) {
        return false;
    }
    if ins.voice == Voice::Dmc {
        return ins.sample == n.sample.map(|(a, l)| [a, l]);
    }
    // Over the whole note: the instrument's last values hold.
    let k = n.volume.len().max(1);
    let mut close = 0;
    for i in 0..k {
        if ins.duty_at(i) != n.duty.get(i).copied().unwrap_or(0)
            || ins.pitch_at(i) != n.pitch.get(i).copied().unwrap_or(0)
        {
            return false;
        }
        if ins
            .volume_at(i)
            .abs_diff(n.volume.get(i).copied().unwrap_or(0))
            <= 1
        {
            close += 1;
        }
    }
    close * 10 >= k * 9
}

/// Clusters notes into instruments, appending new ones to `instruments`
/// (so a set's songs share them). Returns each note's instrument.
pub fn instruments(notes: &[RawNote], instruments: &mut Vec<Instrument>) -> Vec<u16> {
    // Longest first, so an instrument's envelope is its longest note's.
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(notes[i].len()), i));
    let mut of = vec![0u16; notes.len()];
    for i in order {
        let n = &notes[i];
        let at = match instruments.iter().position(|ins| fits(ins, n)) {
            Some(k) => k,
            None => {
                let voice = Voice::of(n.channel);
                let count = instruments.iter().filter(|x| x.voice == voice).count();
                instruments.push(Instrument {
                    name: format!("{}-{}", voice_name(voice), count + 1),
                    voice,
                    role: None,
                    volume: trim(&n.volume),
                    duty: if voice == Voice::Triangle || voice == Voice::Dmc {
                        Vec::new()
                    } else {
                        trim(&n.duty)
                    },
                    pitch: if n.pitch.iter().all(|&p| p == 0) {
                        Vec::new()
                    } else {
                        trim(&n.pitch)
                    },
                    sample: n.sample.map(|(a, l)| [a, l]),
                    notes: 0,
                });
                instruments.len() - 1
            }
        };
        instruments[at].notes += 1;
        of[i] = at as u16;
    }
    of
}

/// An envelope without the repeats of its last value at the end (the last
/// value holds anyway).
fn trim<T: Copy + PartialEq>(v: &[T]) -> Vec<T> {
    let mut n = v.len();
    while n > 1 && v[n - 1] == v[n - 2] {
        n -= 1;
    }
    v[..n].to_vec()
}

fn voice_name(v: Voice) -> &'static str {
    match v {
        Voice::Pulse => "pulse",
        Voice::Triangle => "triangle",
        Voice::Noise => "noise",
        Voice::Dmc => "dmc",
    }
}

/// A raw note as a song note: its pitch, and the fine offset that keeps its
/// exact period.
pub fn to_note(n: &RawNote, instrument: u16, region: Region) -> Note {
    let (p, fine) = match n.channel {
        Channel::Noise | Channel::Dmc => (n.period as u8 & 0x0F, 0),
        c => {
            let m = pitch(c, n.period, region)
                .unwrap_or(0.0)
                .round()
                .clamp(0.0, 127.0);
            let exact = crate::apu::period_of(c, m, region);
            (m as u8, (i32::from(n.period) - i32::from(exact)) as i16)
        }
    };
    Note {
        at: 0,
        len: n.len() as u32,
        pitch: p,
        fine,
        instrument,
    }
}

/// The tempo grid: frames per row, as the start-distance divisor rule says.
pub fn tempo(starts: &[u64]) -> Tempo {
    let Some(&first) = starts.iter().min() else {
        return Tempo {
            frames_per_row: 1,
            rows_per_pattern: 16,
        };
    };
    let mut best = 1;
    for d in 2..=32u64 {
        let hit = starts.iter().filter(|&&s| (s - first) % d == 0).count();
        if hit * 10 >= starts.len() * 9 {
            best = d;
        }
    }
    Tempo {
        frames_per_row: best as u32,
        rows_per_pattern: 16,
    }
}

/// The loop: (start, period) such that every frame's state from `start`
/// repeats `period` frames later to the end, with at least one period of
/// repeat. Masked frames match anything.
pub fn find_loop(states: &[[ChannelState; 5]], mask: &[Vec<bool>; 5]) -> Option<(usize, usize)> {
    let n = states.len();
    let key = |f: usize, c: usize| {
        let s = states[f][c];
        if s.volume == 0 {
            (0u16, 0u8, 0u8)
        } else {
            (s.period, s.volume, s.duty)
        }
    };
    let same =
        |a: usize, b: usize| (0..5).all(|c| mask[c][a] || mask[c][b] || key(a, c) == key(b, c));
    // Frames with any sound, so silence doesn't count as a loop.
    let busy = |f: usize| (0..5).any(|c| !mask[c][f] && states[f][c].volume > 0);
    for p in MIN_LOOP..=n / 2 {
        // Walk back from the end while frames match p later.
        let mut start = n - p;
        while start > 0 && same(start - 1, start - 1 + p) {
            start -= 1;
        }
        if n - p - start >= p && (start..start + p).filter(|&f| busy(f)).count() * 2 >= p {
            return Some((start, p));
        }
    }
    None
}

/// A sound request in a capture's frames (relative to its first).
#[derive(Debug, Clone)]
pub struct Req {
    pub frame: u64,
    pub request: Request,
    /// Music (a song starts) or an effect.
    pub kind: Kind,
}

/// Whether a request starts music: its interface or name says music or
/// song.
pub fn is_music(via: &str, name: &str) -> bool {
    let l = format!("{via} {name}").to_ascii_lowercase();
    ["music", "song", "bgm", "mus_"]
        .iter()
        .any(|w| l.contains(w))
}

/// A channel an interface's name points at.
pub fn named_channel(via: &str) -> Option<Channel> {
    let l = via.to_ascii_lowercase();
    if l.contains("square1") || l.contains("sq1") || l.contains("pulse1") {
        Some(Channel::Pulse1)
    } else if l.contains("square2") || l.contains("sq2") || l.contains("pulse2") {
        Some(Channel::Pulse2)
    } else if l.contains("noise") {
        Some(Channel::Noise)
    } else if l.contains("tri") {
        Some(Channel::Triangle)
    } else if l.contains("dpcm") || l.contains("dmc") {
        Some(Channel::Dmc)
    } else {
        None
    }
}

/// An effect's claim: the channel and frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub request: usize,
    pub channel: Channel,
    pub from: u64,
    pub to: u64,
}

/// Which channels and frames each effect request takes (rule 4).
pub fn claims(states: &[[ChannelState; 5]], reqs: &[Req]) -> Vec<Claim> {
    let n = states.len() as u64;
    let onset = |c: Channel, f: u64| {
        let s = states[f as usize][c.index()];
        let before = if f == 0 {
            0
        } else {
            states[f as usize - 1][c.index()].volume
        };
        s.volume > 0 && (s.trigger || before == 0)
    };
    let mut out: Vec<Claim> = Vec::new();
    for (ri, r) in reqs.iter().enumerate() {
        if r.kind != Kind::Sfx || r.frame >= n {
            continue;
        }
        let window = r.frame..(r.frame + CLAIM_WITHIN + 1).min(n);
        let named = named_channel(&r.request.via);
        let chans: Vec<(Channel, u64)> = Channel::ALL
            .into_iter()
            .filter(|&c| named.is_none_or(|x| x == c))
            .filter_map(|c| window.clone().find(|&f| onset(c, f)).map(|f| (c, f)))
            .collect();
        for (c, from) in chans {
            // Until the next request on this channel, silence, or the cap.
            let next = reqs
                .iter()
                .skip(ri + 1)
                .find(|q| {
                    q.kind == Kind::Sfx
                        && q.frame > r.frame
                        && named_channel(&q.request.via).is_none_or(|x| x == c)
                })
                .map_or(n, |q| q.frame);
            let cap = (from + MAX_EFFECT).min(next).min(n);
            // The channel's duty before the request, when it differs from
            // the effect's: the effect ends when the duty comes back.
            let ci = c.index();
            let before = (r.frame > 0)
                .then(|| states[r.frame as usize - 1][ci])
                .filter(|s| s.volume > 0)
                .map(|s| s.duty)
                .filter(|&d| d != states[from as usize][ci].duty);
            let mut to = from;
            while to < cap
                && states[to as usize][ci].volume > 0
                && before.is_none_or(|d| states[to as usize][ci].duty != d)
            {
                to += 1;
            }
            if to > from {
                out.push(Claim {
                    request: ri,
                    channel: c,
                    from,
                    to,
                });
            }
        }
    }
    out
}

/// What a capture is cut into: a song per music request (or the whole
/// capture when there's none) and an effect per distinct effect request.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub name: String,
    pub recording: Option<String>,
    /// The capture's first frame in the recording.
    pub first_frame: u64,
    /// The level the capture plays, for its music.
    pub level: Option<String>,
}

/// Builds a set from a capture's frame states and its requests.
pub fn capture(
    region: Region,
    states: &[[ChannelState; 5]],
    reqs: &[Req],
    opts: &Options,
) -> SongSet {
    let mut set = SongSet::new(region);
    let n = states.len();
    let cl = claims(states, reqs);
    let mut mask: [Vec<bool>; 5] = Default::default();
    for m in &mut mask {
        *m = vec![false; n];
    }
    for c in &cl {
        for f in c.from..c.to {
            mask[c.channel.index()][f as usize] = true;
        }
    }

    // Music segments: from each music request to the next, or the whole.
    let mut cuts: Vec<(u64, Option<&Req>)> = reqs
        .iter()
        .filter(|r| r.kind == Kind::Music && (r.frame as usize) < n)
        .map(|r| (r.frame, Some(r)))
        .collect();
    if cuts.first().is_none_or(|c| c.0 > 0) {
        cuts.insert(0, (0, None));
    }
    let mut used_names: BTreeMap<String, usize> = BTreeMap::new();
    for (k, &(from, req)) in cuts.iter().enumerate() {
        let to = cuts.get(k + 1).map_or(n as u64, |c| c.0);
        let seg = &states[from as usize..to as usize];
        let seg_mask: [Vec<bool>; 5] =
            std::array::from_fn(|c| mask[c][from as usize..to as usize].to_vec());
        let raw = notes(seg, Some(&seg_mask));
        if raw.is_empty() {
            continue;
        }
        let base = match req {
            Some(r) => r.request.name.clone(),
            None => opts.name.clone(),
        };
        let name = unique(&mut used_names, &slug(&base));
        let mut song = build(&mut set, region, &raw, &name, Kind::Music, seg.len() as u64);
        let masked_frames: u64 = seg_mask
            .iter()
            .map(|m| m.iter().filter(|&&x| x).count() as u64)
            .sum();
        let mut confidence = 1.0 - masked_frames as f64 / (5 * seg.len().max(1)) as f64;
        if let Some((start, period)) = find_loop(seg, &seg_mask) {
            song.loop_frame = Some(start as u64);
            song.frames = (start + period) as u64;
            // Notes after the loop's end repeat earlier ones.
            for t in &mut song.tracks {
                let mut placed = t.placed(song.tempo);
                placed.retain(|p| p.start < song.frames);
                for p in &mut placed {
                    let end = song.frames - p.start;
                    p.note.len = p.note.len.min(end as u32);
                }
                *t = Track::from_placed(t.channel, &placed, song.tempo, song.frames);
            }
        } else {
            song.notes.push(
                "no loop found in the capture: it may be shorter than one pass of the song".into(),
            );
            confidence *= 0.8;
        }
        let shift = fit_grid(&mut song);
        song.confidence = round2(confidence);
        song.request = req.map(|r| r.request.clone());
        song.level = opts.level.clone();
        song.captured = opts.recording.as_ref().map(|r| Captured {
            recording: r.clone(),
            frames: [opts.first_frame + from + shift, opts.first_frame + to],
        });
        set.songs.push(song);
    }

    // Effects: one per distinct request, from its first clean capture.
    let mut seen: BTreeMap<(String, u8), ()> = BTreeMap::new();
    for (ri, r) in reqs.iter().enumerate() {
        if r.kind != Kind::Sfx {
            continue;
        }
        let key = (r.request.via.clone(), r.request.id);
        if seen.contains_key(&key) {
            continue;
        }
        let mine: Vec<&Claim> = cl.iter().filter(|c| c.request == ri).collect();
        if mine.is_empty() {
            continue;
        }
        let from = mine.iter().map(|c| c.from).min().unwrap_or(0);
        let to = mine.iter().map(|c| c.to).max().unwrap_or(from);
        let seg = &states[from as usize..to as usize];
        let only: [Vec<bool>; 5] = std::array::from_fn(|c| {
            (from..to)
                .map(|f| {
                    !mine
                        .iter()
                        .any(|m| m.channel.index() == c && f >= m.from && f < m.to)
                })
                .collect()
        });
        let raw = notes(seg, Some(&only));
        if raw.is_empty() {
            continue;
        }
        seen.insert(key, ());
        let name = unique(&mut used_names, &slug(&r.request.name));
        let mut song = build(&mut set, region, &raw, &name, Kind::Sfx, (to - from).max(1));
        // An effect cut short by the next request may be incomplete.
        let cut = mine.iter().any(|m| m.to - m.from >= MAX_EFFECT);
        song.confidence = if named_channel(&r.request.via).is_some() {
            0.9
        } else {
            0.7
        } * if cut { 0.7 } else { 1.0 };
        song.confidence = round2(song.confidence);
        song.request = Some(r.request.clone());
        song.captured = opts.recording.as_ref().map(|rec| Captured {
            recording: rec.clone(),
            frames: [opts.first_frame + from, opts.first_frame + to],
        });
        set.songs.push(song);
    }
    roles(&mut set);
    set
}

/// Drops a song's leading silence and picks rows per pattern so the loop
/// and the end fall on pattern boundaries when some common length does.
/// Returns the frames dropped.
pub fn fit_grid(song: &mut Song) -> u64 {
    let old = song.tempo;
    let mut all: Vec<(Channel, Vec<Placed>)> = song
        .tracks
        .iter()
        .map(|t| (t.channel, t.placed(old)))
        .collect();
    let shift = all
        .iter()
        .filter_map(|(_, p)| p.first().map(|x| x.start))
        .min()
        .unwrap_or(0);
    for (_, p) in &mut all {
        for x in p.iter_mut() {
            x.start -= shift;
        }
    }
    song.frames = song.frames.saturating_sub(shift).max(1);
    song.loop_frame = song.loop_frame.map(|l| l.saturating_sub(shift));
    let fpr = u64::from(old.frames_per_row.max(1));
    let fits = |rows: u64| {
        let pf = rows * fpr;
        song.frames.is_multiple_of(pf) && song.loop_frame.is_none_or(|l| l.is_multiple_of(pf))
    };
    let rows = [16u64, 32, 12, 24, 8, 64, 48, 4, 2, 1]
        .into_iter()
        .find(|&r| fits(r))
        .unwrap_or(16);
    let tempo = Tempo {
        frames_per_row: old.frames_per_row,
        rows_per_pattern: rows as u32,
    };
    song.tracks = all
        .iter()
        .map(|(c, p)| Track::from_placed(*c, p, tempo, song.frames))
        .collect();
    song.tempo = tempo;
    shift
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// A song from raw notes in segment time.
fn build(
    set: &mut SongSet,
    region: Region,
    raw: &[RawNote],
    name: &str,
    kind: Kind,
    frames: u64,
) -> Song {
    let ins = instruments(raw, &mut set.instruments);
    let starts: Vec<u64> = raw.iter().map(|n| n.start).collect();
    let tempo = tempo(&starts);
    let mut tracks = Vec::new();
    for c in Channel::ALL {
        let placed: Vec<Placed> = raw
            .iter()
            .zip(&ins)
            .filter(|(n, _)| n.channel == c)
            .map(|(n, &i)| Placed {
                start: n.start,
                note: to_note(n, i, region),
            })
            .collect();
        if !placed.is_empty() {
            tracks.push(Track::from_placed(c, &placed, tempo, frames));
        }
    }
    Song {
        name: name.into(),
        kind,
        origin: Origin::Log,
        confidence: 1.0,
        driver: None,
        frames,
        loop_frame: None,
        tempo,
        tracks,
        request: None,
        event: None,
        level: None,
        state: None,
        captured: None,
        notes: Vec::new(),
    }
}

/// Default roles: an effect's instruments are `sfx`; in music, noise and
/// DMC are `drums`, the triangle `bass`, and of the pulses the one with
/// the higher mean pitch `lead`, the other `harmony`.
pub fn roles(set: &mut SongSet) {
    let mut role: BTreeMap<u16, (&'static str, u32)> = BTreeMap::new();
    let mut vote = |i: u16, r: &'static str| {
        let e = role.entry(i).or_insert((r, 0));
        if e.0 == r {
            e.1 += 1;
        } else if e.1 == 0 {
            *e = (r, 1);
        } else {
            e.1 -= 1;
        }
    };
    for s in &set.songs {
        let mean = |c: Channel| {
            let p = s.track(c).map(|t| t.placed(s.tempo)).unwrap_or_default();
            if p.is_empty() {
                0.0
            } else {
                p.iter().map(|x| f64::from(x.note.pitch)).sum::<f64>() / p.len() as f64
            }
        };
        let lead = if mean(Channel::Pulse1) >= mean(Channel::Pulse2) {
            Channel::Pulse1
        } else {
            Channel::Pulse2
        };
        for t in &s.tracks {
            let r = match (s.kind, t.channel) {
                (Kind::Sfx, _) => "sfx",
                (_, Channel::Noise | Channel::Dmc) => "drums",
                (_, Channel::Triangle) => "bass",
                (_, c) if c == lead => "lead",
                _ => "harmony",
            };
            for p in t.placed(s.tempo) {
                vote(p.note.instrument, r);
            }
        }
    }
    for (i, (r, _)) in role {
        if let Some(ins) = set.instruments.get_mut(usize::from(i))
            && ins.role.is_none()
        {
            ins.role = Some(r.into());
        }
    }
}

/// A name usable as a model path segment and a file name.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_owned();
    if out.is_empty() { "song".into() } else { out }
}

fn unique(used: &mut BTreeMap<String, usize>, base: &str) -> String {
    let n = used.entry(base.to_owned()).or_insert(0);
    *n += 1;
    if *n == 1 {
        base.to_owned()
    } else {
        format!("{base}_{n}")
    }
}

/// The words of a name: split at `_`, `.`, `-`, spaces, digits and lower
/// to upper case changes, lowercased.
pub fn words(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in s.chars() {
        if !ch.is_ascii_alphabetic() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if ch.is_ascii_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        prev_lower = ch.is_ascii_lowercase();
        cur.push(ch.to_ascii_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.retain(|w| {
        w.len() > 1 && !["sfx", "snd", "sound", "music", "queue", "mus", "se"].contains(&w.as_str())
    });
    out
}

/// Words that name the same happening, for any game: an effect's name
/// word on the left, the model event or state words it stands for on the
/// right.
pub const SYNONYMS: &[(&str, &[&str])] = &[
    ("stomp", &["stomped"]),
    ("kick", &["kicked"]),
    ("coin", &["collected"]),
    ("grab", &["collected", "lifted"]),
    ("powerup", &["collected"]),
    ("item", &["collected"]),
    ("death", &["died", "dying", "dead"]),
    ("die", &["died", "dying"]),
    ("dead", &["died", "dying"]),
    ("hurt", &["hurt"]),
    ("damage", &["hurt", "hurt_player"]),
    ("hit", &["hit", "hurt"]),
    ("throw", &["thrown"]),
    ("lift", &["lifted"]),
    ("over", &["game_over", "over"]),
    ("victory", &["won", "level_end", "goal", "clear"]),
    ("flag", &["goal", "reached", "level_end"]),
    ("clear", &["won", "level_end", "clear"]),
];

fn matches(word: &str, target: &str) -> bool {
    let t = target.to_ascii_lowercase();
    let parts: Vec<&str> = t.split(['.', '_', '-']).collect();
    let stem = |w: &str| w.chars().take(4).collect::<String>();
    if parts
        .iter()
        .any(|p| p.len() >= 4 && word.len() >= 4 && stem(p) == stem(word))
    {
        return true;
    }
    SYNONYMS
        .iter()
        .filter(|(w, _)| *w == word)
        .any(|(_, to)| to.iter().any(|x| parts.contains(x) || t == *x))
}

/// The candidate (an event or a rule state) a sound's name points at, if
/// exactly one fits best.
pub fn bind(name: &str, candidates: &[String]) -> Option<String> {
    let ws = words(name);
    let mut best: Vec<(usize, &String)> = candidates
        .iter()
        .map(|c| (ws.iter().filter(|w| matches(w, c)).count(), c))
        .filter(|(n, _)| *n > 0)
        .collect();
    best.sort_by_key(|(n, c)| (std::cmp::Reverse(*n), (*c).clone()));
    match best.as_slice() {
        [] => None,
        [(_, c)] => Some((*c).clone()),
        [(a, c), (b, _), ..] if a > b => Some((*c).clone()),
        _ => None,
    }
}
