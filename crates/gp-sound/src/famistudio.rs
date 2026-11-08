//! FamiStudio text files (FamiStudio's "FamiStudio Text" export and
//! import, as documented at https://famistudio.org/doc/export/): the
//! faithful external music editor, as Pixelorama is for sprites.
//!
//! Export writes the project in FamiStudio's tempo mode with each song's
//! frames per row as its note length, every note at its own frame, and
//! instruments as volume, duty and pitch envelopes. Import reads such a
//! file back and checks it against what the NES's five channels can play:
//! the 2A03 channels only, one note at a time per channel, periods and
//! volumes in range, envelopes of at most 255 frames.
//!
//! Three readings of FamiStudio's format aren't confirmed against the
//! program yet and are rows in docs/open-questions.md: note times in
//! FamiStudio tempo mode are frames; a pitch envelope's positive values
//! raise the pitch (a smaller period); noise note `n` semitones above C0
//! plays period index 15 - (n mod 16).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::apu::{Channel, Sample};
use crate::song::{Instrument, Kind, Note, Origin, Placed, Song, SongSet, Tempo, Track, Voice};

/// The FamiStudio version written in the header; FamiStudio reads text
/// files of its own major and minor version.
pub const DEFAULT_VERSION: &str = "4.5.0";

const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// A MIDI note's FamiStudio name (MIDI 12 is C0).
pub fn note_name(midi: u8) -> String {
    let m = i32::from(midi);
    format!("{}{}", NAMES[(m % 12) as usize], m / 12 - 1)
}

pub fn parse_note_name(s: &str) -> Option<u8> {
    let split = s.find(|c: char| c.is_ascii_digit() || c == '-')?;
    let (n, o) = s.split_at(split);
    let i = NAMES.iter().position(|x| *x == n)? as i32;
    let o: i32 = o.parse().ok()?;
    u8::try_from((o + 1) * 12 + i).ok()
}

fn channel_type(c: Channel) -> &'static str {
    match c {
        Channel::Pulse1 => "Square1",
        Channel::Pulse2 => "Square2",
        Channel::Triangle => "Triangle",
        Channel::Noise => "Noise",
        Channel::Dmc => "DPCM",
    }
}

fn q(s: &str) -> String {
    s.replace('"', "\"\"")
}

fn join<T: ToString>(v: &[T]) -> String {
    v.iter().map(T::to_string).collect::<Vec<_>>().join(",")
}

/// What an export couldn't keep exactly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Losses {
    pub clamped_pitch: usize,
    pub long_envelopes: usize,
    pub dmc_without_bytes: usize,
}

/// Writes a set as a FamiStudio text project.
pub fn export(set: &SongSet, name: &str, version: &str) -> (String, Losses) {
    let mut s = String::new();
    let mut loss = Losses::default();
    let _ = writeln!(
        s,
        "Project Version=\"{}\" TempoMode=\"FamiStudio\" Name=\"{}\" Author=\"\" Copyright=\"\"",
        q(version),
        q(name)
    );
    // DPCM: one sample per DMC instrument with bytes, mapped from C1 up.
    let mut dpcm: BTreeMap<u16, (String, u8)> = BTreeMap::new();
    for (i, ins) in set.instruments.iter().enumerate() {
        let Some([addr, len]) = ins.sample else {
            continue;
        };
        let Some(smp) = set.samples.iter().find(|x| x.addr == addr) else {
            loss.dmc_without_bytes += 1;
            continue;
        };
        let bytes = &smp.bytes[..usize::from(len).min(smp.bytes.len())];
        let mut hex = String::new();
        for b in bytes {
            let _ = write!(hex, "{b:02X}");
        }
        let note = 24 + dpcm.len() as u8;
        let _ = writeln!(s, "\tDPCMSample Name=\"{}\" Data=\"{hex}\"", q(&ins.name));
        dpcm.insert(i as u16, (ins.name.clone(), note));
    }
    for (sample, note) in dpcm.values() {
        let _ = writeln!(
            s,
            "\tDPCMMapping Note=\"{}\" Sample=\"{}\" Pitch=\"15\" Loop=\"False\"",
            note_name(*note),
            q(sample)
        );
    }
    for ins in &set.instruments {
        if ins.voice == Voice::Dmc {
            continue;
        }
        let _ = writeln!(s, "\tInstrument Name=\"{}\"", q(&ins.name));
        let env = |s: &mut String, kind: &str, v: Vec<i32>, extra: &str, loss: &mut Losses| {
            let mut v = v;
            if v.len() > 255 {
                v.truncate(255);
                loss.long_envelopes += 1;
            }
            let _ = writeln!(
                s,
                "\t\tEnvelope Type=\"{kind}\" Length=\"{}\"{extra} Values=\"{}\"",
                v.len(),
                join(&v)
            );
        };
        let vol: Vec<i32> = if ins.volume.is_empty() {
            vec![15]
        } else {
            ins.volume.iter().map(|&x| i32::from(x)).collect()
        };
        env(&mut s, "Volume", vol, "", &mut loss);
        if !ins.duty.is_empty() {
            env(
                &mut s,
                "DutyCycle",
                ins.duty.iter().map(|&x| i32::from(x)).collect(),
                "",
                &mut loss,
            );
        }
        if !ins.pitch.is_empty() {
            // FamiStudio's values are -64 to 63, positive up: absolute
            // offsets when they fit, else each frame's change (Relative),
            // which a sweep's steady slide keeps small.
            let abs: Vec<i32> = ins.pitch.iter().map(|&x| -i32::from(x)).collect();
            let fits = |v: &[i32]| v.iter().all(|x| (-64..=63).contains(x));
            if fits(&abs) {
                env(&mut s, "Pitch", abs, " Relative=\"False\"", &mut loss);
            } else {
                let mut prev = 0;
                let rel: Vec<i32> = abs
                    .iter()
                    .map(|&x| {
                        let d = x - prev;
                        prev = x;
                        d
                    })
                    .collect();
                if !fits(&rel) {
                    loss.clamped_pitch += rel.iter().filter(|x| !(-64..=63).contains(*x)).count();
                }
                // A last 0, so the slide stops where the note's does
                // however FamiStudio treats a relative envelope's end.
                let rel = rel
                    .into_iter()
                    .map(|x| x.clamp(-64, 63))
                    .chain([0])
                    .collect();
                env(&mut s, "Pitch", rel, " Relative=\"True\"", &mut loss);
            }
        }
    }
    for song in &set.songs {
        let t = song.tempo;
        let pf = u64::from(t.pattern_frames().max(1));
        let cols = song.frames.div_ceil(pf).max(1);
        let lp = song.loop_frame.map_or(-1, |l| (l / pf) as i64);
        let _ = writeln!(
            s,
            "\tSong Name=\"{}\" Length=\"{cols}\" LoopPoint=\"{lp}\" PatternLength=\"{}\" BeatLength=\"4\" NoteLength=\"{}\" Groove=\"{}\" GroovePaddingMode=\"Middle\"",
            q(&song.name),
            t.rows_per_pattern,
            t.frames_per_row,
            t.frames_per_row
        );
        // A last column shorter than the rest.
        let tail = song.frames - (cols - 1) * pf;
        if tail != pf {
            let rows = tail.div_ceil(u64::from(t.frames_per_row.max(1)));
            let _ = writeln!(
                s,
                "\t\tPatternCustomSettings Time=\"{}\" Length=\"{rows}\" NoteLength=\"{}\" BeatLength=\"4\" Groove=\"{}\" GroovePaddingMode=\"Middle\"",
                cols - 1,
                t.frames_per_row,
                t.frames_per_row
            );
        }
        for tr in &song.tracks {
            let _ = writeln!(s, "\t\tChannel Type=\"{}\"", channel_type(tr.channel));
            for (pi, p) in tr.patterns.iter().enumerate() {
                let _ = writeln!(s, "\t\t\tPattern Name=\"{}-{pi}\"", q(&song.name));
                for n in &p.notes {
                    let ins = &set.instruments[usize::from(n.instrument)];
                    let value = match tr.channel {
                        Channel::Noise => note_name(12 + (15 - (n.pitch & 15))),
                        Channel::Dmc => match dpcm.get(&n.instrument) {
                            Some((_, note)) => note_name(*note),
                            None => continue,
                        },
                        _ => note_name(n.pitch),
                    };
                    let fine = if n.fine != 0 && tr.channel != Channel::Noise {
                        format!(" FinePitch=\"{}\"", (-i32::from(n.fine)).clamp(-128, 127))
                    } else {
                        String::new()
                    };
                    let ins_attr = if tr.channel == Channel::Dmc {
                        String::new()
                    } else {
                        format!(" Instrument=\"{}\"", q(&ins.name))
                    };
                    let _ = writeln!(
                        s,
                        "\t\t\t\tNote Time=\"{}\" Value=\"{value}\" Duration=\"{}\"{ins_attr}{fine}",
                        n.at, n.len
                    );
                }
            }
            for (col, p) in tr.order.iter().enumerate() {
                if let Some(p) = p {
                    let _ = writeln!(
                        s,
                        "\t\t\tPatternInstance Time=\"{col}\" Pattern=\"{}-{p}\"",
                        q(&song.name)
                    );
                }
            }
        }
    }
    (s, loss)
}

/// One line: its object and attributes.
fn parse_line(line: &str) -> Result<(String, BTreeMap<String, String>), String> {
    let line = line.trim();
    let (obj, mut rest) = line.split_once(' ').unwrap_or((line, ""));
    let mut attrs = BTreeMap::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let eq = rest
            .find("=\"")
            .ok_or_else(|| format!("bad attribute in `{line}`"))?;
        let key = rest[..eq].to_owned();
        let mut val = String::new();
        let mut chars = rest[eq + 2..].char_indices().peekable();
        let mut end = None;
        while let Some((i, c)) = chars.next() {
            if c == '"' {
                if chars.peek().map(|x| x.1) == Some('"') {
                    val.push('"');
                    chars.next();
                } else {
                    end = Some(eq + 2 + i + 1);
                    break;
                }
            } else {
                val.push(c);
            }
        }
        let end = end.ok_or_else(|| format!("an unclosed quote in `{line}`"))?;
        attrs.insert(key, val);
        rest = &rest[end..];
    }
    Ok((obj.to_owned(), attrs))
}

/// What an import found against the NES's limits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Validation {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

fn num<T: std::str::FromStr>(a: &BTreeMap<String, String>, k: &str) -> Option<T> {
    a.get(k).and_then(|v| v.parse().ok())
}

fn values(a: &BTreeMap<String, String>) -> Vec<i32> {
    a.get("Values")
        .map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_default()
}

/// Reads a FamiStudio text project into a set, and checks it.
pub fn import(text: &str, region: crate::apu::Region) -> (SongSet, Validation) {
    let mut set = SongSet::new(region);
    let mut v = Validation::default();
    let mut ins_index: BTreeMap<String, u16> = BTreeMap::new();
    let mut samples: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut mapping: BTreeMap<u8, String> = BTreeMap::new();
    // The song being read: header, current channel, its patterns and
    // instances.
    struct Cur {
        song: Song,
        channel: Option<Channel>,
        patterns: BTreeMap<String, Vec<Note>>,
        pattern: Option<String>,
        instances: Vec<(u64, String)>,
        tracks: Vec<(Channel, BTreeMap<String, Vec<Note>>, Vec<(u64, String)>)>,
    }
    fn close_channel(c: &mut Cur) {
        if let Some(ch) = c.channel.take() {
            c.tracks.push((
                ch,
                std::mem::take(&mut c.patterns),
                std::mem::take(&mut c.instances),
            ));
        }
    }
    fn finish(c: &mut Cur, v: &mut Validation) -> Song {
        close_channel(c);
        let t = c.song.tempo;
        let pf = u64::from(t.pattern_frames().max(1));
        for (ch, pats, inst) in std::mem::take(&mut c.tracks) {
            let mut placed: Vec<Placed> = Vec::new();
            for (col, name) in inst {
                match pats.get(&name) {
                    Some(notes) => placed.extend(notes.iter().map(|n| Placed {
                        start: col * pf + u64::from(n.at),
                        note: n.clone(),
                    })),
                    None => v
                        .errors
                        .push(format!("{}: no pattern `{name}`", c.song.name)),
                }
            }
            placed.sort_by_key(|p| p.start);
            // One note at a time: a note ends where the next starts.
            for i in 1..placed.len() {
                let next = placed[i].start;
                let p = &mut placed[i - 1];
                if p.start + u64::from(p.note.len) > next {
                    p.note.len = (next - p.start) as u32;
                    v.warnings.push(format!(
                        "{} {}: a note overlapping the next was cut at frame {next}",
                        c.song.name,
                        ch.name()
                    ));
                }
            }
            placed.retain(|p| p.note.len > 0);
            c.song
                .tracks
                .push(Track::from_placed(ch, &placed, t, c.song.frames));
        }
        c.song.clone()
    }
    let mut cur: Option<Cur> = None;
    for (ln, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (obj, a) = match parse_line(line) {
            Ok(x) => x,
            Err(e) => {
                v.errors.push(format!("line {}: {e}", ln + 1));
                continue;
            }
        };
        match obj.as_str() {
            "Project" => {
                if let Some(x) = a.get("Expansion") {
                    v.errors.push(format!(
                        "the project uses expansion audio ({x}); the NES has the 2A03's five channels only"
                    ));
                }
                if a.get("TempoMode").is_some_and(|m| m != "FamiStudio") {
                    v.errors
                        .push("only FamiStudio tempo mode is read (frames per note)".into());
                }
            }
            "DPCMSample" => {
                let hex = a.get("Data").cloned().unwrap_or_default();
                let bytes: Vec<u8> = (0..hex.len() / 2)
                    .filter_map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok())
                    .collect();
                samples.insert(a.get("Name").cloned().unwrap_or_default(), bytes);
            }
            "DPCMMapping" => {
                if let (Some(n), Some(smp)) = (
                    a.get("Note").and_then(|n| parse_note_name(n)),
                    a.get("Sample"),
                ) {
                    mapping.insert(n, smp.clone());
                }
            }
            "Instrument" => {
                if a.contains_key("Expansion") {
                    v.errors.push(format!(
                        "instrument `{}` is an expansion instrument",
                        a.get("Name").cloned().unwrap_or_default()
                    ));
                }
                let name = a.get("Name").cloned().unwrap_or_default();
                ins_index.insert(name.clone(), set.instruments.len() as u16);
                set.instruments.push(Instrument {
                    name,
                    voice: Voice::Pulse,
                    role: None,
                    volume: Vec::new(),
                    duty: Vec::new(),
                    pitch: Vec::new(),
                    sample: None,
                    notes: 0,
                });
            }
            "Envelope" => {
                let Some(ins) = set.instruments.last_mut() else {
                    v.errors.push(format!(
                        "line {}: an envelope outside an instrument",
                        ln + 1
                    ));
                    continue;
                };
                let vals = values(&a);
                if vals.len() > 255 {
                    v.errors
                        .push(format!("{}: an envelope longer than 255 frames", ins.name));
                }
                match a.get("Type").map(String::as_str) {
                    Some("Volume") => {
                        if vals.iter().any(|x| !(0..=15).contains(x)) {
                            v.errors.push(format!("{}: volume outside 0-15", ins.name));
                        }
                        ins.volume = vals.iter().map(|&x| x.clamp(0, 15) as u8).collect();
                    }
                    Some("DutyCycle") => {
                        if vals.iter().any(|x| !(0..=3).contains(x)) {
                            v.errors.push(format!("{}: duty outside 0-3", ins.name));
                        }
                        ins.duty = vals.iter().map(|&x| x.clamp(0, 3) as u8).collect();
                    }
                    Some("Pitch") => {
                        if a.get("Relative").is_some_and(|r| r == "True") {
                            // Relative: each value adds to the last.
                            let mut acc = 0i32;
                            ins.pitch = vals
                                .iter()
                                .map(|&x| {
                                    acc += x;
                                    (-acc) as i16
                                })
                                .collect();
                            // The trailing rest the export adds.
                            while ins.pitch.len() > 1
                                && ins.pitch[ins.pitch.len() - 1] == ins.pitch[ins.pitch.len() - 2]
                            {
                                ins.pitch.pop();
                            }
                        } else {
                            ins.pitch = vals.iter().map(|&x| (-x) as i16).collect();
                        }
                    }
                    Some(other) => v
                        .warnings
                        .push(format!("{}: a {other} envelope isn't kept", ins.name)),
                    None => v
                        .errors
                        .push(format!("line {}: an envelope with no type", ln + 1)),
                }
            }
            "Arpeggio" => v
                .warnings
                .push("arpeggios aren't kept; notes play their own pitch".into()),
            "Song" => {
                if let Some(mut c) = cur.take() {
                    let s = finish(&mut c, &mut v);
                    set.songs.push(s);
                }
                let fpr: u32 = num(&a, "NoteLength").unwrap_or(1).max(1);
                let rows: u32 = num(&a, "PatternLength").unwrap_or(16).max(1);
                let cols: u64 = num(&a, "Length").unwrap_or(1);
                let lp: i64 = num(&a, "LoopPoint").unwrap_or(-1);
                let tempo = Tempo {
                    frames_per_row: fpr,
                    rows_per_pattern: rows,
                };
                let pf = u64::from(tempo.pattern_frames());
                cur = Some(Cur {
                    song: Song {
                        name: a.get("Name").cloned().unwrap_or_else(|| "song".into()),
                        kind: Kind::Music,
                        origin: Origin::Import,
                        confidence: 1.0,
                        driver: None,
                        frames: cols * pf,
                        loop_frame: (lp >= 0).then(|| lp as u64 * pf),
                        tempo,
                        tracks: Vec::new(),
                        request: None,
                        event: None,
                        level: None,
                        state: None,
                        captured: None,
                        notes: vec!["imported from a FamiStudio text file".into()],
                    },
                    channel: None,
                    patterns: BTreeMap::new(),
                    pattern: None,
                    instances: Vec::new(),
                    tracks: Vec::new(),
                });
            }
            "PatternCustomSettings" => {
                // Only a shorter last column is kept: it ends the song.
                let Some(c) = cur.as_mut() else { continue };
                let t = c.song.tempo;
                let pf = u64::from(t.pattern_frames());
                let col: u64 = num(&a, "Time").unwrap_or(0);
                let rows: u64 = num(&a, "Length").unwrap_or(u64::from(t.rows_per_pattern));
                let fpr: u64 = num(&a, "NoteLength").unwrap_or(u64::from(t.frames_per_row));
                if (col + 1) * pf == c.song.frames {
                    c.song.frames = col * pf + rows * fpr;
                } else {
                    v.warnings.push(format!(
                        "{}: a custom pattern length before the last column isn't kept",
                        c.song.name
                    ));
                }
            }
            "Channel" => {
                let Some(c) = cur.as_mut() else { continue };
                close_channel(c);
                let ch = match a.get("Type").map(String::as_str) {
                    Some("Square1") => Some(Channel::Pulse1),
                    Some("Square2") => Some(Channel::Pulse2),
                    Some("Triangle") => Some(Channel::Triangle),
                    Some("Noise") => Some(Channel::Noise),
                    Some("DPCM") => Some(Channel::Dmc),
                    other => {
                        v.errors.push(format!(
                            "channel {} isn't one of the NES's five",
                            other.unwrap_or("?")
                        ));
                        None
                    }
                };
                c.channel = ch;
            }
            "Pattern" => {
                if let Some(c) = cur.as_mut() {
                    let n = a.get("Name").cloned().unwrap_or_default();
                    c.patterns.entry(n.clone()).or_default();
                    c.pattern = Some(n);
                }
            }
            "Note" => {
                let Some(c) = cur.as_mut() else { continue };
                let (Some(ch), Some(pn)) = (c.channel, c.pattern.clone()) else {
                    continue;
                };
                let Some(value) = a.get("Value") else {
                    continue;
                };
                if value == "Stop" {
                    // Ends the note before it.
                    let at: u32 = num(&a, "Time").unwrap_or(0);
                    if let Some(last) = c.patterns.get_mut(&pn).and_then(|p| p.last_mut())
                        && last.at + last.len > at
                    {
                        last.len = at.saturating_sub(last.at);
                    }
                    continue;
                }
                let Some(midi) = parse_note_name(value) else {
                    v.errors.push(format!("line {}: note `{value}`", ln + 1));
                    continue;
                };
                let at: u32 = num(&a, "Time").unwrap_or(0);
                let len: u32 = num(&a, "Duration").unwrap_or(1).max(1);
                let fine: i16 = -num::<i16>(&a, "FinePitch").unwrap_or(0);
                let (pitch, instrument) = match ch {
                    Channel::Noise => (15 - (midi.saturating_sub(12) & 15), a.get("Instrument")),
                    Channel::Dmc => (15, None),
                    _ => (midi, a.get("Instrument")),
                };
                let instrument = match ch {
                    Channel::Dmc => {
                        let Some(name) = mapping.get(&midi) else {
                            v.errors
                                .push(format!("line {}: no DPCM sample on {value}", ln + 1));
                            continue;
                        };
                        dmc_instrument(&mut set, &mut ins_index, name, &samples)
                    }
                    _ => match instrument.and_then(|n| ins_index.get(n)) {
                        Some(&i) => i,
                        None => {
                            v.errors
                                .push(format!("line {}: the note has no known instrument", ln + 1));
                            continue;
                        }
                    },
                };
                // Range on its channel.
                let note = Note {
                    at,
                    len,
                    pitch,
                    fine,
                    instrument,
                };
                let period = note.period(ch, region);
                match ch {
                    Channel::Pulse1 | Channel::Pulse2 if period < 8 => v.errors.push(format!(
                        "{}: {value} is too high for a pulse channel",
                        c.song.name
                    )),
                    Channel::Triangle if period < 2 => v.errors.push(format!(
                        "{}: {value} is too high for the triangle",
                        c.song.name
                    )),
                    _ if ch != Channel::Noise && ch != Channel::Dmc && period >= 0x7FF => {
                        v.errors.push(format!(
                            "{}: {value} is too low for {}",
                            c.song.name,
                            ch.name()
                        ))
                    }
                    _ => {}
                }
                c.patterns.entry(pn).or_default().push(note);
            }
            "PatternInstance" => {
                if let Some(c) = cur.as_mut() {
                    let t: u64 = num(&a, "Time").unwrap_or(0);
                    if let Some(p) = a.get("Pattern") {
                        c.instances.push((t, p.clone()));
                    }
                }
            }
            other => v
                .warnings
                .push(format!("line {}: `{other}` isn't read", ln + 1)),
        }
    }
    if let Some(mut c) = cur.take() {
        let s = finish(&mut c, &mut v);
        set.songs.push(s);
    }
    // Each instrument's voice is that of the channels that play it.
    let mut voice: BTreeMap<u16, Voice> = BTreeMap::new();
    for s in &set.songs {
        for t in &s.tracks {
            for p in t.placed(s.tempo) {
                let want = Voice::of(t.channel);
                match voice.get(&p.note.instrument) {
                    Some(&x) if x != want => v.errors.push(format!(
                        "instrument `{}` plays on both a {x:?} and a {want:?} channel; give each its own",
                        set.instruments[usize::from(p.note.instrument)].name
                    )),
                    _ => {
                        voice.insert(p.note.instrument, want);
                    }
                }
            }
        }
    }
    for (i, vc) in voice {
        set.instruments[usize::from(i)].voice = vc;
    }
    for (name, bytes) in samples {
        if let Some(ins) = set.instruments.iter().find(|i| i.name == name)
            && let Some([addr, _]) = ins.sample
        {
            set.samples.push(Sample { addr, bytes });
        }
    }
    if let Err(e) = set.check() {
        v.errors.push(e);
    }
    (set, v)
}

fn dmc_instrument(
    set: &mut SongSet,
    index: &mut BTreeMap<String, u16>,
    name: &str,
    samples: &BTreeMap<String, Vec<u8>>,
) -> u16 {
    if let Some(&i) = index.get(name) {
        return i;
    }
    let len = samples.get(name).map_or(1, Vec::len) as u16;
    // Samples are placed from $C000 in 64-byte steps, in import order.
    let addr = set
        .instruments
        .iter()
        .filter_map(|i| i.sample)
        .map(|[a, l]| a + l.div_ceil(64) * 64)
        .max()
        .unwrap_or(0xC000);
    let i = set.instruments.len() as u16;
    set.instruments.push(Instrument {
        name: name.to_owned(),
        voice: Voice::Dmc,
        role: Some("drums".into()),
        volume: Vec::new(),
        duty: Vec::new(),
        pitch: Vec::new(),
        sample: Some([addr, len]),
        notes: 0,
    });
    index.insert(name.to_owned(), i);
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apu::Region;
    use crate::capture::{self, Options};

    #[test]
    fn note_names() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(70), "A#4");
        assert_eq!(parse_note_name("A#4"), Some(70));
        assert_eq!(parse_note_name("C0"), Some(12));
        assert_eq!(parse_note_name("H2"), None);
    }

    #[test]
    fn round_trip_keeps_every_note() {
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
        let (text, loss) = export(&set, "made up", DEFAULT_VERSION);
        assert_eq!(loss, Losses::default());
        assert!(text.starts_with("Project Version=\"4.5.0\" TempoMode=\"FamiStudio\""));
        let (back, v) = import(&text, Region::Ntsc);
        assert!(v.ok(), "{v:?}");
        let a = &set.songs[0];
        let b = &back.songs[0];
        assert_eq!(a.frames, b.frames);
        assert_eq!(a.loop_frame, b.loop_frame);
        for c in a.channels() {
            let pa: Vec<_> = a.track(c).unwrap().placed(a.tempo);
            let pb: Vec<_> = b.track(c).unwrap().placed(b.tempo);
            assert_eq!(pa.len(), pb.len(), "{c:?}");
            for (x, y) in pa.iter().zip(&pb) {
                assert_eq!(
                    (x.start, x.note.pitch, x.note.len, x.note.fine),
                    (y.start, y.note.pitch, y.note.len, y.note.fine)
                );
                let (ix, iy) = (
                    &set.instruments[usize::from(x.note.instrument)],
                    &back.instruments[usize::from(y.note.instrument)],
                );
                assert_eq!(
                    (&ix.volume, &ix.duty, &ix.pitch),
                    (&iy.volume, &iy.duty, &iy.pitch)
                );
            }
        }
    }

    #[test]
    fn import_refuses_what_the_nes_cant_play() {
        let text = "Project Version=\"4.5.0\" TempoMode=\"FamiStudio\" Expansion=\"VRC6\"\n\
            \tInstrument Name=\"a\"\n\t\tEnvelope Type=\"Volume\" Length=\"1\" Values=\"20\"\n\
            \tSong Name=\"s\" Length=\"1\" LoopPoint=\"0\" PatternLength=\"16\" BeatLength=\"4\" NoteLength=\"6\"\n\
            \t\tChannel Type=\"VRC6Saw\"\n\
            \t\tChannel Type=\"Square1\"\n\t\t\tPattern Name=\"p\"\n\
            \t\t\t\tNote Time=\"0\" Value=\"C0\" Duration=\"10\" Instrument=\"a\"\n\
            \t\t\t\tNote Time=\"4\" Value=\"C4\" Duration=\"10\" Instrument=\"a\"\n\
            \t\t\tPatternInstance Time=\"0\" Pattern=\"p\"\n";
        let (_, v) = import(text, Region::Ntsc);
        let all = v.errors.join("\n");
        assert!(all.contains("expansion audio"), "{all}");
        assert!(all.contains("volume outside 0-15"), "{all}");
        assert!(all.contains("VRC6Saw"), "{all}");
        assert!(all.contains("too low for pulse1"), "{all}");
        assert!(
            v.warnings.iter().any(|w| w.contains("overlapping")),
            "{v:?}"
        );
    }
}
