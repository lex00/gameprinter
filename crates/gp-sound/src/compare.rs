//! Two runs' APU writes held to each other frame by frame: a rebuild that
//! keeps a game's sound driver as state against the original's register
//! log, or any two logs. Two measures, for any game:
//!
//! | Measure | Same when |
//! |---|---|
//! | Writes | The frame's writes are the same registers and values in the same order |
//! | Channels | Every channel's period, volume, duty and trigger at the frame's end are the same, each side's writes played through the APU model ([`crate::apu`]) from power-on |
//!
//! The channels measure forgives what a listener can't hear: a register
//! written twice, writes in another order within the frame, and writes the
//! game makes outside its driver that change nothing.

use crate::apu::Region;
use crate::apu::{Channel, ChannelState};
use crate::log::{ApuLog, states_of};

/// One frame's writes, each as the register's offset from $4000 and the
/// value, in order: the original's and the model's.
pub type Frame = (Vec<(u8, u8)>, Vec<(u8, u8)>);

/// How two runs' writes agree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Agreement {
    /// The frames compared.
    pub frames: u64,
    /// Frames whose writes are the same.
    pub writes: u64,
    /// Frames whose every channel's state is the same.
    pub channels: u64,
    /// Each channel's frames the same, in [`Channel::ALL`]'s order.
    pub by_channel: [u64; 5],
    /// The first frame (an index into the frames given) whose writes
    /// differ, with both sides' writes.
    pub first_writes: Option<(usize, Frame)>,
    /// The first frame whose channel states differ, the channel, and both
    /// sides' state of it.
    pub first_channel: Option<(usize, Channel, ChannelState, ChannelState)>,
    /// The longest run of frames with every channel the same.
    pub longest: u64,
    /// Each frame whose channel states differ, and the channels that do.
    pub apart: Vec<(usize, Vec<Channel>)>,
    /// Each frame whose writes differ.
    pub writes_apart: Vec<usize>,
}

/// Each channel's state at the end of each frame of `writes`, the writes
/// of a frame in order at its start. A run compared from the middle of a
/// game doesn't know what the APU held before its first frame, so both
/// sides start from power-on with the four tone channels turned on
/// ($4015 = $0F), as a game that plays sound keeps them.
fn states(region: Region, writes: &[&[(u8, u8)]]) -> Vec<[ChannelState; 5]> {
    let on: &[(u8, u8)] = &[(0x15, 0x0F)];
    let timeline: Vec<(u64, u8, u8)> = std::iter::once((0u64, on))
        .chain(
            writes
                .iter()
                .enumerate()
                .map(|(f, w)| (region.frame_start(f as u64) + 1, *w)),
        )
        .flat_map(|(at, w)| {
            w.iter()
                .enumerate()
                .map(move |(i, &(r, v))| (at + i as u64, r, v))
        })
        .collect();
    states_of(region, &timeline, writes.len() as u64, &[])
}

/// What a listener hears of a channel's state: its trigger only while it
/// sounds, since a restart of a silent channel makes no sound.
fn heard(s: &ChannelState) -> (u16, u8, u8, bool) {
    if s.volume == 0 {
        (0, 0, 0, false)
    } else {
        (s.period, s.volume, s.duty, s.trigger)
    }
}

/// The two runs' writes held to each other, frame by frame.
pub fn compare(region: Region, frames: &[Frame]) -> Agreement {
    let orig: Vec<&[(u8, u8)]> = frames.iter().map(|f| f.0.as_slice()).collect();
    let model: Vec<&[(u8, u8)]> = frames.iter().map(|f| f.1.as_slice()).collect();
    let (so, sm) = (states(region, &orig), states(region, &model));
    let mut a = Agreement {
        frames: frames.len() as u64,
        ..Agreement::default()
    };
    let mut run = 0;
    for (i, f) in frames.iter().enumerate() {
        if f.0 == f.1 {
            a.writes += 1;
        } else {
            a.writes_apart.push(i);
            if a.first_writes.is_none() {
                a.first_writes = Some((i, f.clone()));
            }
        }
        let mut all = true;
        let mut off = Vec::new();
        for (c, ch) in Channel::ALL.iter().enumerate() {
            if heard(&so[i][c]) == heard(&sm[i][c]) {
                a.by_channel[c] += 1;
            } else {
                all = false;
                off.push(*ch);
                if a.first_channel.is_none() {
                    a.first_channel = Some((i, *ch, so[i][c], sm[i][c]));
                }
            }
        }
        if !off.is_empty() {
            a.apart.push((i, off));
        }
        if all {
            a.channels += 1;
            run += 1;
            a.longest = a.longest.max(run);
        } else {
            run = 0;
        }
    }
    a
}

/// [`Agreement::apart`] as runs of frames next to each other with the
/// same channels apart: (first, last, channels).
pub fn apart_spans(a: &Agreement) -> Vec<(usize, usize, Vec<Channel>)> {
    let mut out: Vec<(usize, usize, Vec<Channel>)> = Vec::new();
    for (i, chs) in &a.apart {
        match out.last_mut() {
            Some(last) if last.1 + 1 == *i && &last.2 == chs => last.1 = *i,
            _ => out.push((*i, *i, chs.clone())),
        }
    }
    out
}

/// Two logs' frames side by side: the original's frame `f` against the
/// model's `f - offset`, over the frames both have.
pub fn pair_logs(original: &ApuLog, model: &ApuLog, offset: i64) -> (u64, Vec<Frame>) {
    let by_frame = |log: &ApuLog| {
        let mut m: std::collections::BTreeMap<u64, Vec<(u8, u8)>> = Default::default();
        for w in &log.writes {
            m.entry(w.frame).or_default().push((w.reg, w.value));
        }
        m
    };
    let (o, m) = (by_frame(original), by_frame(model));
    let first = original
        .first
        .max(u64::try_from(model.first as i64 + offset).unwrap_or(0));
    let end = original
        .end
        .min(u64::try_from(model.end as i64 + offset).unwrap_or(0));
    let frames = (first..end)
        .map(|f| {
            let mf = u64::try_from(f as i64 - offset).unwrap_or(u64::MAX);
            (
                o.get(&f).cloned().unwrap_or_default(),
                m.get(&mf).cloned().unwrap_or_default(),
            )
        })
        .collect();
    (first, frames)
}

/// Lines for people: the two measures, each channel, and the first frame
/// each measure differs on, numbered by `name` (an index into the frames
/// given -> how the report names that frame).
pub fn lines(a: &Agreement, name: &dyn Fn(usize) -> String) -> Vec<String> {
    let mut out = vec![format!(
        "sound: APU writes the same on {} of {} frames, every channel's state on {} (longest run {})",
        a.writes, a.frames, a.channels, a.longest
    )];
    let per: Vec<String> = Channel::ALL
        .iter()
        .zip(a.by_channel)
        .map(|(c, n)| format!("{} {n}", c.name()))
        .collect();
    out.push(format!("  by channel: {}", per.join(", ")));
    if let Some((i, ch, o, m)) = &a.first_channel {
        out.push(format!(
            "  first channel split at {}: {} original period {} volume {} duty {}{}, model period {} volume {} duty {}{}",
            name(*i),
            ch.name(),
            o.period,
            o.volume,
            o.duty,
            if o.trigger { " (restarted)" } else { "" },
            m.period,
            m.volume,
            m.duty,
            if m.trigger { " (restarted)" } else { "" },
        ));
    }
    if let Some((i, (o, m))) = &a.first_writes {
        let show = |w: &[(u8, u8)]| {
            w.iter()
                .map(|(r, v)| format!("{:04X}={v:02X}", 0x4000 + u16::from(*r)))
                .collect::<Vec<_>>()
                .join(" ")
        };
        out.push(format!(
            "  first writes split at {}: original [{}], model [{}]",
            name(*i),
            show(o),
            show(m)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(period: u16) -> Vec<(u8, u8)> {
        vec![
            (0x15, 0x0F),
            (0x00, 0xBF),
            (0x02, (period & 0xFF) as u8),
            (0x03, (period >> 8) as u8 | 0x08),
        ]
    }

    #[test]
    fn the_same_writes_agree_on_every_frame() {
        let frames: Vec<Frame> = (0..10)
            .map(|i| {
                let w = if i % 3 == 0 {
                    note(200 + i)
                } else {
                    Vec::new()
                };
                (w.clone(), w)
            })
            .collect();
        let a = compare(Region::Ntsc, &frames);
        assert_eq!(
            (a.frames, a.writes, a.channels, a.longest),
            (10, 10, 10, 10)
        );
        assert!(a.first_writes.is_none() && a.first_channel.is_none());
    }

    #[test]
    fn a_repeated_write_differs_in_writes_and_not_in_channels() {
        let mut twice = note(300);
        twice.push((0x00, 0xBF));
        let frames = vec![(note(300), twice)];
        let a = compare(Region::Ntsc, &frames);
        assert_eq!((a.writes, a.channels), (0, 1));
        assert_eq!(a.first_writes.as_ref().map(|f| f.0), Some(0));
    }

    #[test]
    fn another_pitch_differs_on_its_channel_from_its_frame() {
        let frames = vec![
            (note(300), note(300)),
            (Vec::new(), Vec::new()),
            (note(300), note(310)),
            (Vec::new(), Vec::new()),
        ];
        let a = compare(Region::Ntsc, &frames);
        assert_eq!(a.channels, 2);
        assert_eq!(a.by_channel[0], 2);
        assert_eq!(a.by_channel[1], 4);
        let (i, ch, o, m) = a.first_channel.unwrap();
        assert_eq!((i, ch, o.period, m.period), (2, Channel::Pulse1, 300, 310));
    }

    #[test]
    fn logs_pair_by_frame_with_an_offset() {
        let log = |first: u64, at: u64| ApuLog {
            region: Region::Ntsc,
            source: None,
            recording: None,
            first,
            end: first + 5,
            timed: false,
            writes: vec![crate::log::Write {
                frame: at,
                cycle: 0,
                reg: 0x15,
                value: 0x0F,
            }],
            samples: Vec::new(),
        };
        let (first, frames) = pair_logs(&log(10, 12), &log(7, 9), 3);
        assert_eq!(first, 10);
        assert_eq!(frames.len(), 5);
        assert_eq!(compare(Region::Ntsc, &frames).writes, 5);
    }
}
