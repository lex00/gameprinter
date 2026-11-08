//! Reaching any frame of a stored recording: restore the nearest checkpoint
//! at or before it, then replay the input log from there. With checkpoints
//! every 60 frames that is a restore and at most 59 frames of replay.
//!
//! A recording made by another runtime has checkpoints this one can't
//! restore, so it's replayed from power-on instead, and what that reaches
//! may differ from what was recorded. [`Cursor::matches_recording`] says
//! whether it does.

use gp_evidence::frame_hashes::FrameHash;
use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::RecordingManifest;
use gp_evidence::{InputLog, Store};

use crate::recording::{self, RecordingError};
use crate::{FrameOutcome, Nes, NesError};

#[derive(Debug, thiserror::Error)]
pub enum SeekError {
    #[error(transparent)]
    Recording(#[from] RecordingError),
    #[error(transparent)]
    Nes(#[from] NesError),
    #[error("frame {frame} is past the end of the recording, which has {frames} frames")]
    PastEnd { frame: u64, frames: u64 },
    #[error(
        "the cursor is at frame {at} and only goes forward; open the recording again for frame {frame}"
    )]
    Backwards { at: u64, frame: u64 },
}

/// A console positioned in a recording, with its input log.
pub struct Cursor {
    nes: Nes,
    input: InputLog,
    hashes: Vec<FrameHash>,
    /// The checkpoint it started from, or `None` for power-on.
    pub checkpoint: Option<u64>,
    /// Frames replayed since the start, checkpoint or power-on.
    pub replayed: u64,
    /// Whether another runtime made the recording and its checkpoints
    /// weren't used (recording::Lineage).
    pub stale: bool,
}

/// Opens a recording at `frame`: the state after that many frames, which
/// is the state input log entry `frame` is applied to.
pub fn open(
    store: &Store,
    manifest: &RecordingManifest,
    rom: &[u8],
    frame: u64,
) -> Result<Cursor, SeekError> {
    if frame > manifest.frames {
        return Err(SeekError::PastEnd {
            frame,
            frames: manifest.frames,
        });
    }
    let (input, hashes) = recording::load(store, manifest)?;
    let mut nes = Nes::power_on(rom, manifest.power_on)?;
    let mut checkpoint = manifest
        .checkpoints
        .iter()
        .filter(|c| c.frame <= frame)
        .max_by_key(|c| c.frame);
    if let Some(cp) = checkpoint {
        let held = match cp.frame {
            0 => [0; 4],
            f => input.frames[f as usize - 1].pads,
        };
        if !recording::restore_if_usable(&mut nes, rom, store, manifest, cp, held, &hashes)? {
            checkpoint = None;
        }
    }
    // Every recording has a checkpoint at frame 0, so none used means this
    // runtime may not restore them.
    let stale = checkpoint.is_none();
    let mut cursor = Cursor {
        nes,
        input,
        hashes,
        checkpoint: checkpoint.map(|c| c.frame),
        replayed: 0,
        stale,
    };
    cursor.advance_to(frame)?;
    Ok(cursor)
}

impl Cursor {
    pub fn nes(&self) -> &Nes {
        &self.nes
    }

    pub fn nes_mut(&mut self) -> &mut Nes {
        &mut self.nes
    }

    pub fn into_nes(self) -> Nes {
        self.nes
    }

    pub fn frame(&self) -> u64 {
        self.nes.frame()
    }

    /// Frames in the recording.
    pub fn frames(&self) -> u64 {
        self.input.frames.len() as u64
    }

    /// The input log entry applied to the state at `frame`, if the
    /// recording has one.
    pub fn input(&self, frame: u64) -> Option<InputFrame> {
        self.input.frames.get(frame as usize).copied()
    }

    /// Runs the next frame of the recording.
    pub fn step(&mut self) -> Result<FrameOutcome, SeekError> {
        let frame = self.frame();
        let input = self.input(frame).ok_or(SeekError::PastEnd {
            frame: frame + 1,
            frames: self.frames(),
        })?;
        self.replayed += 1;
        Ok(self.nes.step(input)?)
    }

    /// Replays forward to `frame`.
    pub fn advance_to(&mut self, frame: u64) -> Result<(), SeekError> {
        if frame < self.frame() {
            return Err(SeekError::Backwards {
                at: self.frame(),
                frame,
            });
        }
        while self.frame() < frame {
            self.step()?;
        }
        Ok(())
    }

    /// Whether the console's state is the one recorded for its frame: the
    /// same RAM and canonical state hashes.
    pub fn matches_recording(&self) -> Result<bool, NesError> {
        Ok(matches(&self.nes, &self.hashes)? != Some(false))
    }

    /// The recorded frame hashes, frames 0 to the last.
    pub fn hashes(&self) -> &[FrameHash] {
        &self.hashes
    }
}

/// Whether `nes` is in the state `hashes` recorded for its frame, or `None`
/// past the last frame, where nothing was recorded.
pub fn matches(nes: &Nes, hashes: &[FrameHash]) -> Result<Option<bool>, NesError> {
    let Some(recorded) = hashes.get(nes.frame() as usize) else {
        return Ok(None);
    };
    Ok(Some(
        nes.wram_hash() == recorded.ram && nes.state_hash()? == recorded.state,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::{RecordRequest, record};
    use crate::testrom;
    use gp_evidence::input_log::generate;
    use gp_evidence::manifest::{PowerOn, Provenance, SourceRef};

    fn recorded(store: &Store, rom: &[u8]) -> RecordingManifest {
        let input = generate(200, &["a@10-20", "right@50-"]).unwrap();
        record(
            store,
            RecordRequest {
                rom,
                source: SourceRef::NesRom {
                    sha256: gp_evidence::source::identify_nes(rom).unwrap().sha256,
                    mapper: 0,
                },
                source_id: None,
                power_on: PowerOn::default(),
                input: &input,
                checkpoint_interval: 30,
                provenance: Provenance::Scripted,
                name: "seek".into(),
                created_at: "2026-10-27T00:00:00Z".into(),
                notes: None,
            },
        )
        .unwrap()
        .manifest
    }

    #[test]
    fn reaches_every_frame_from_the_nearest_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let rom = testrom::build();
        let manifest = recorded(&store, &rom);
        for frame in [0, 1, 29, 30, 31, 95, 199, 200] {
            let cursor = open(&store, &manifest, &rom, frame).unwrap();
            assert_eq!(cursor.frame(), frame);
            assert_eq!(cursor.checkpoint, Some(frame / 30 * 30));
            assert_eq!(cursor.replayed, frame % 30);
            assert!(cursor.matches_recording().unwrap(), "frame {frame}");
            assert!(!cursor.stale);
        }
        let mut cursor = open(&store, &manifest, &rom, 100).unwrap();
        cursor.advance_to(150).unwrap();
        assert!(cursor.matches_recording().unwrap());
        assert!(matches!(
            cursor.advance_to(149),
            Err(SeekError::Backwards { at: 150, .. })
        ));
        assert!(matches!(
            open(&store, &manifest, &rom, 201),
            Err(SeekError::PastEnd { frame: 201, .. })
        ));
        let mut end = open(&store, &manifest, &rom, 200).unwrap();
        assert!(matches!(end.step(), Err(SeekError::PastEnd { .. })));
    }

    #[test]
    fn uses_checkpoints_from_a_patch_only_runtime_that_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let rom = testrom::build();
        let mut manifest = recorded(&store, &rom);
        let (head, _) = manifest.runtime.version.split_once(" + patch ").unwrap();
        manifest.runtime.version = format!("{head} + patch 000000000000)");
        let cursor = open(&store, &manifest, &rom, 75).unwrap();
        assert!(!cursor.stale);
        assert_eq!((cursor.checkpoint, cursor.replayed), (Some(60), 15));
        assert!(cursor.matches_recording().unwrap());
        // One that doesn't round-trip is passed over for power-on.
        let mut bad = store.get(&manifest.checkpoints[2].snapshot).unwrap();
        bad.push(0);
        manifest.checkpoints[2].snapshot = store.put_bytes(&bad).unwrap();
        let cursor = open(&store, &manifest, &rom, 75).unwrap();
        assert!(cursor.stale);
        assert_eq!((cursor.checkpoint, cursor.replayed), (None, 75));
        assert!(cursor.matches_recording().unwrap());
    }

    #[test]
    fn replays_another_runtimes_recording_from_power_on() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let rom = testrom::build();
        let mut manifest = recorded(&store, &rom);
        manifest.runtime.version = "0 (an older gp-nes)".into();
        let cursor = open(&store, &manifest, &rom, 75).unwrap();
        assert!(cursor.stale);
        assert_eq!((cursor.checkpoint, cursor.replayed), (None, 75));
        assert!(cursor.matches_recording().unwrap());
    }
}
