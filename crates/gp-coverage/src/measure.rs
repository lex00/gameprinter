//! Measuring one recording: replay it with the [`Observer`] watching each
//! instruction and the definition reading RAM after each frame, and check
//! that the replay reaches the recorded frame hashes.
//!
//! A recording made by this runtime replays in segments that start at its
//! checkpoints, on several threads at once, each checked against the frame
//! hash where it ends. One made by another `gp-nes` replays from power-on,
//! checked at every checkpoint interval and at its last frame. Either way,
//! coverage of a replay that doesn't match its recording is refused.

use std::collections::BTreeMap;

use gp_evidence::manifest::{RecordingManifest, SourceRef};
use gp_evidence::{Hash, Store};
use gp_nes::seek::{self, Cursor, SeekError};

use crate::definition::Definition;
use crate::{COVERAGE_VERSION, Coverage, DefinitionRef, Seen, combo_name};
use gp_events::{Footprint as Observer, Layout, LayoutError as PassError};

#[derive(Debug, thiserror::Error)]
pub enum MeasureError {
    #[error(transparent)]
    Seek(#[from] SeekError),
    #[error(transparent)]
    Pass(#[from] PassError),
    #[error(
        "the replay doesn't reach the recorded state at frame {frame}, so its coverage would describe another run; `gp replay` shows where it diverges"
    )]
    Diverged { frame: u64 },
    #[error("{0}")]
    Other(String),
}

/// What one segment of the replay saw.
struct Segment {
    observer: Observer,
    screens: BTreeMap<Vec<i64>, Seen>,
    objects: BTreeMap<i64, Seen>,
}

/// Measures a recording. `threads` caps how many segments run at once.
pub fn measure(
    store: &Store,
    id: Hash,
    manifest: &RecordingManifest,
    rom: &[u8],
    definition: Option<&Definition>,
    threads: usize,
) -> Result<Coverage, MeasureError> {
    let SourceRef::NesRom { sha256, mapper } = &manifest.source else {
        return Err(MeasureError::Other(
            "a model recording has no ROM, so coverage can't measure it".into(),
        ));
    };
    let first = seek::open(store, manifest, rom, 0)?;
    let layout = Layout::of(first.nes().bus())?;
    let stale = first.stale;
    let frames = manifest.frames;

    let segments: Vec<Segment> = if stale || threads <= 1 || manifest.checkpoints.is_empty() {
        let check_every = if stale {
            u64::from(manifest.checkpoint_interval.max(1))
        } else {
            u64::MAX
        };
        vec![run(first, layout, frames, check_every, definition)?]
    } else {
        drop(first);
        let starts = split(manifest, threads);
        let mut ends: Vec<u64> = starts[1..].to_vec();
        ends.push(frames);
        std::thread::scope(|scope| {
            let handles: Vec<_> = starts
                .iter()
                .zip(&ends)
                .map(|(&start, &end)| {
                    scope.spawn(move || -> Result<Segment, MeasureError> {
                        let cursor = seek::open(store, manifest, rom, start)?;
                        run(cursor, layout, end, u64::MAX, definition)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .map_err(|_| MeasureError::Other("a coverage thread panicked".into()))?
                })
                .collect::<Result<Vec<_>, _>>()
        })?
    };

    let mut observer = Observer::new(layout);
    let mut screens: BTreeMap<Vec<i64>, Seen> = BTreeMap::new();
    let mut objects: BTreeMap<i64, Seen> = BTreeMap::new();
    for s in &segments {
        observer.merge(&s.observer).map_err(MeasureError::Other)?;
        for (key, seen) in &s.screens {
            merge_seen(screens.entry(key.clone()), seen);
        }
        for (id, seen) in &s.objects {
            merge_seen(objects.entry(*id), seen);
        }
    }

    let cursor = seek::open(store, manifest, rom, 0)?;
    let mut inputs: [BTreeMap<String, u64>; 2] = Default::default();
    for f in 0..frames {
        let input = cursor.input(f).expect("frame within the input log");
        if input.polled {
            for (port, counts) in inputs.iter_mut().enumerate() {
                *counts.entry(combo_name(input.pads[port])).or_default() += 1;
            }
        }
    }

    let mut cov = Coverage {
        coverage_version: COVERAGE_VERSION,
        sha256: sha256.clone(),
        mapper: *mapper,
        recordings: vec![id],
        frames,
        instructions: observer.instructions,
        outside_rom: observer.outside_rom,
        windows: observer
            .windows
            .iter()
            .map(|mask| {
                (0..64u16)
                    .filter(|p| mask & (1 << p) != 0)
                    .map(|p| p << 10)
                    .collect()
            })
            .collect(),
        executed: observer.executed,
        starts: observer.starts,
        ram_written: observer.ram_written,
        prg_ram_written: observer.prg_ram_written,
        inputs,
        definition: definition.map(|d| DefinitionRef {
            source: d.source.clone(),
            hash: d.hash,
        }),
        screens: BTreeMap::new(),
        levels: BTreeMap::new(),
        objects: BTreeMap::new(),
    };
    if let Some(def) = definition {
        if let Some(s) = &def.screens {
            for (key, seen) in &screens {
                cov.screens.insert(s.screen_name(key), *seen);
                merge_seen(cov.levels.entry(s.level_name(key)), seen);
            }
            // A level's frames add up over its screens; it was seen in one
            // recording however many screens it has.
            for seen in cov.levels.values_mut() {
                seen.recordings = 1;
            }
        }
        for (id, seen) in &objects {
            merge_seen(cov.objects.entry(def.object_name(*id)), seen);
        }
        for seen in cov.objects.values_mut() {
            seen.recordings = 1;
        }
    }
    Ok(cov)
}

fn merge_seen<K: Ord>(entry: std::collections::btree_map::Entry<'_, K, Seen>, seen: &Seen) {
    entry
        .and_modify(|e| {
            e.frames += seen.frames;
            e.first = match (e.first, seen.first) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        })
        .or_insert(*seen);
}

/// Segment start frames: 0 and checkpoint frames, about `threads` segments
/// of similar length.
fn split(manifest: &RecordingManifest, threads: usize) -> Vec<u64> {
    let mut frames: Vec<u64> = manifest
        .checkpoints
        .iter()
        .map(|c| c.frame)
        .filter(|&f| f > 0 && f < manifest.frames)
        .collect();
    frames.sort_unstable();
    frames.dedup();
    let target = manifest.frames.div_ceil(threads as u64).max(1);
    let mut starts = vec![0];
    for f in frames {
        if f >= starts.last().unwrap() + target {
            starts.push(f);
        }
    }
    starts
}

/// Runs `cursor` to frame `end`, watching every instruction, and checks the
/// state against the recording every `check_every` frames and at `end`.
fn run(
    mut cursor: Cursor,
    layout: Layout,
    end: u64,
    check_every: u64,
    definition: Option<&Definition>,
) -> Result<Segment, MeasureError> {
    let mut seg = Segment {
        observer: Observer::new(layout),
        screens: BTreeMap::new(),
        objects: BTreeMap::new(),
    };
    let screens = definition.and_then(|d| d.screens.as_ref());
    let objects = definition.and_then(|d| d.objects.as_ref());
    let mut ids = Vec::new();
    seg.observer.start(cursor.nes_mut().bus_mut_for_observers());
    while cursor.frame() < end {
        let frame = cursor.frame() + 1;
        let input = cursor.input(frame - 1).ok_or(SeekError::PastEnd {
            frame,
            frames: cursor.frames(),
        })?;
        let observer = &mut seg.observer;
        cursor
            .nes_mut()
            .step_with(input, |bus| observer.before(bus))
            .map_err(SeekError::from)?;
        observer.interrupt(cursor.nes_mut().bus_mut_for_observers());
        let ram = cursor.nes().wram();
        if let Some(s) = screens
            && let Some(key) = s.key(ram)
        {
            seg.screens
                .entry(key)
                .and_modify(|e| e.add_frame(frame))
                .or_insert(Seen::at(frame));
        }
        if let Some(o) = objects {
            o.active(ram, &mut ids);
            ids.sort_unstable();
            ids.dedup();
            for id in &ids {
                seg.objects
                    .entry(*id)
                    .and_modify(|e| e.add_frame(frame))
                    .or_insert(Seen::at(frame));
            }
        }
        if (frame.is_multiple_of(check_every) || frame == end) && !matches(&cursor)? {
            return Err(MeasureError::Diverged { frame });
        }
    }
    Ok(seg)
}

fn matches(cursor: &Cursor) -> Result<bool, MeasureError> {
    Ok(seek::matches(cursor.nes(), cursor.hashes()).map_err(SeekError::from)? != Some(false))
}
