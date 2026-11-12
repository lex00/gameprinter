//! The replay that reads a recording's code activity: gp-events'
//! collector with its activity observer ([`gp_events::activity`]), run in
//! segments from the recording's checkpoints (docs/gaps.md, "Code
//! signals"). For `gp gaps`.

use gp_evidence::Store;
use gp_evidence::manifest::RecordingManifest;
use gp_nes::seek::{self, Cursor, SeekError};

use crate::measure::MeasureError;
pub use gp_events::activity::{Activity, CallStats, FrameActivity, NO_ROM, Write};
use gp_events::{Bits, Collector, Layout};

/// A recording's code activity: frame `f`'s entry is what ran while frame
/// `f` ran, so entry 0 (power-on) is empty.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub frames: Vec<FrameActivity>,
    /// Every instruction start run, by PRG offset.
    pub starts: Bits,
    pub prg_rom: usize,
    /// Every `JSR` target's calls, by its PRG offset.
    pub calls: std::collections::BTreeMap<u32, CallStats>,
}

/// Replays a recording with [`Activity`] watching, in segments from its
/// checkpoints on up to `threads` threads, each checked against the frame
/// hash where it ends. `marks` as for [`Activity::new`], `watch` as for
/// [`Activity::watching`]; with `calls_only`, each frame keeps only its
/// watched calls.
pub fn record(
    store: &Store,
    manifest: &RecordingManifest,
    rom: &[u8],
    marks: &Bits,
    watch: &std::collections::BTreeSet<u32>,
    calls_only: bool,
    threads: usize,
) -> Result<Recorded, MeasureError> {
    let first = seek::open(store, manifest, rom, 0)?;
    let layout = Layout::of(first.nes().bus())?;
    let frames = manifest.frames;
    let mut starts: Vec<u64> = vec![0];
    if !first.stale && threads > 1 {
        let target = frames.div_ceil(threads as u64).max(1);
        let mut cps: Vec<u64> = manifest
            .checkpoints
            .iter()
            .map(|c| c.frame)
            .filter(|&f| f > 0 && f < frames)
            .collect();
        cps.sort_unstable();
        for f in cps {
            if f >= starts.last().unwrap() + target {
                starts.push(f);
            }
        }
    }
    drop(first);
    let mut ends: Vec<u64> = starts[1..].to_vec();
    ends.push(frames);
    let parts = std::thread::scope(|scope| {
        let handles: Vec<_> = starts
            .iter()
            .zip(&ends)
            .map(|(&start, &end)| {
                scope.spawn(move || -> Result<Part, MeasureError> {
                    let cursor = seek::open(store, manifest, rom, start)?;
                    run(cursor, layout, end, marks, watch, calls_only)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .map_err(|_| MeasureError::Other("an activity thread panicked".into()))?
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    let mut out = Recorded {
        frames: vec![FrameActivity::default()],
        starts: Bits::new(layout.prg_rom),
        prg_rom: layout.prg_rom,
        calls: Default::default(),
    };
    for (f, s, calls) in parts {
        out.frames.extend(f);
        out.starts.union(&s).map_err(MeasureError::Other)?;
        for (k, v) in calls {
            out.calls.entry(k).or_default().merge(&v);
        }
    }
    Ok(out)
}

type Part = (
    Vec<FrameActivity>,
    Bits,
    std::collections::HashMap<u32, CallStats>,
);

fn run(
    mut cursor: Cursor,
    layout: Layout,
    end: u64,
    marks: &Bits,
    watch: &std::collections::BTreeSet<u32>,
    calls_only: bool,
) -> Result<Part, MeasureError> {
    let mut a = Activity::watching(layout, marks, watch.clone());
    a.calls_only = calls_only;
    // The footprint's instruction starts are the activity's starts: one
    // look at each instruction gives both.
    let mut c = Collector::with_activity(layout, false, a);
    c.start(cursor.nes_mut().bus_mut_for_observers());
    let mut out = Vec::new();
    let every = 600;
    while cursor.frame() < end {
        let frame = cursor.frame() + 1;
        let input = cursor.input(frame - 1).ok_or(SeekError::PastEnd {
            frame,
            frames: cursor.frames(),
        })?;
        cursor
            .nes_mut()
            .step_with(input, |bus| c.before(bus))
            .map_err(SeekError::from)?;
        out.push(
            c.end_frame_activity(cursor.nes_mut().bus_mut_for_observers())
                .expect("watching"),
        );
        if (frame.is_multiple_of(every) || frame == end)
            && seek::matches(cursor.nes(), cursor.hashes()).map_err(SeekError::from)? == Some(false)
        {
            return Err(MeasureError::Diverged { frame });
        }
    }
    let calls = c.activity.take().expect("watching").calls;
    Ok((out, c.footprint.starts, calls))
}
