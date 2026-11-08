//! Checking a baseline (`gp_evidence::baseline`): record its scripted run
//! into a store, then compare the frame hash digest, the lag frame count and
//! the RAM assertions with what the baseline expects.

use gp_evidence::baseline::{Baseline, RamAssert};
use gp_evidence::frame_hashes::{self, FrameHash};
use gp_evidence::manifest::{PowerOn, Provenance, RecordingManifest, SourceRef};
use gp_evidence::{Hash, InputLog, Store};

use crate::Nes;
use crate::recording::{self, Capture, RecordRequest, RecordingError};

#[derive(Debug)]
pub struct BaselineRun {
    /// Hash of the recorded frame hash object.
    pub frame_hashes: Hash,
    pub lag_frames: usize,
    pub frames_per_second: f64,
    /// Each assertion with the value the run produced.
    pub asserts: Vec<(RamAssert, u8)>,
    /// With video rendering on, if asked for: the frame hash digest.
    pub video_frame_hashes: Option<Hash>,
    /// With video rendering on, the first frame whose events differ from
    /// the headless run's, if any.
    pub video_events_differ_at: Option<u64>,
}

impl BaselineRun {
    /// Every way the run differs from the baseline, as sentences.
    pub fn failures(&self, baseline: &Baseline) -> Vec<String> {
        let mut out = Vec::new();
        match baseline.expected_frame_hashes() {
            Ok(want) if want == self.frame_hashes => {}
            _ => out.push(format!(
                "frame hashes are {}, expected {}",
                self.frame_hashes, baseline.frame_hashes
            )),
        }
        if self.lag_frames != baseline.lag_frames {
            out.push(format!(
                "{} lag frames, expected {}",
                self.lag_frames, baseline.lag_frames
            ));
        }
        for (a, actual) in &self.asserts {
            if *actual != a.equals {
                out.push(format!(
                    "{} (${:04X}) at frame {} is {actual}, expected {}",
                    a.label, a.addr, a.frame, a.equals
                ));
            }
        }
        if let Some(video) = self.video_frame_hashes
            && video != self.frame_hashes
        {
            out.push(format!(
                "with video rendering on, frame hashes are {video} instead of {}",
                self.frame_hashes
            ));
        }
        if let Some(frame) = self.video_events_differ_at {
            out.push(format!(
                "with video rendering on, the events of frame {frame} differ from the headless run's"
            ));
        }
        out
    }
}

/// Records `baseline` from `rom` into `store` and reads back what it's
/// checked against. With `video`, also runs it with video rendering on, which
/// takes the PPU's full pixel path instead of the headless sprite-0 path,
/// and compares its frame hashes and every frame's events with the headless
/// run's. The headless recording then keeps an event stream, which changes
/// no hash.
pub fn run(
    store: &Store,
    baseline: &Baseline,
    name: &str,
    rom: &[u8],
    video: bool,
) -> Result<BaselineRun, RecordingError> {
    let input = baseline
        .input()
        .map_err(|e| RecordingError::Corrupt(e.to_string()))?;
    let image = gp_evidence::source::identify_nes(rom)
        .map_err(|e| RecordingError::Corrupt(e.to_string()))?;
    let capture = Capture {
        trace: None,
        events: video.then_some(gp_evidence::chunk::DEFAULT_CHUNK_FRAMES),
    };
    let recorded = recording::record_with(
        store,
        RecordRequest {
            rom,
            source: SourceRef::NesRom {
                sha256: image.sha256,
                mapper: image.header.mapper,
            },
            source_id: baseline.source.clone(),
            power_on: PowerOn::default(),
            input: &input,
            checkpoint_interval: 60,
            name: name.to_owned(),
            provenance: Provenance::Scripted,
            // The store is only for this check, so the time isn't kept.
            created_at: "1970-01-01T00:00:00Z".into(),
            notes: None,
        },
        capture,
    )?;
    let manifest = &recorded.manifest;
    let frame_hashes = manifest
        .frame_hashes
        .ok_or_else(|| RecordingError::Corrupt("the recording has no frame hashes".into()))?;
    let mut asserts = Vec::new();
    for a in &baseline.asserts {
        let ram = ram_at(store, manifest, &input, rom, a.frame)?;
        asserts.push((a.clone(), ram[usize::from(a.addr)]));
    }
    let (video_frame_hashes, video_events_differ_at) = if video {
        let headless = recording::events_by_frame(store, manifest)?;
        let (hashes, differ_at) = run_with_video(rom, &input, &headless)?;
        (Some(Hash::of(&frame_hashes::encode(&hashes))), differ_at)
    } else {
        (None, None)
    };
    Ok(BaselineRun {
        frame_hashes,
        lag_frames: recorded.lag_frames,
        frames_per_second: recorded.frames_per_second,
        asserts,
        video_frame_hashes,
        video_events_differ_at,
    })
}

/// Internal RAM at `frame`, from the nearest checkpoint at or before it.
fn ram_at(
    store: &Store,
    manifest: &RecordingManifest,
    input: &InputLog,
    rom: &[u8],
    frame: u64,
) -> Result<Vec<u8>, RecordingError> {
    let cp = manifest
        .checkpoints
        .iter()
        .rev()
        .find(|c| c.frame <= frame)
        .ok_or_else(|| RecordingError::Corrupt("no checkpoint at frame 0".into()))?;
    let mut nes = Nes::power_on(rom, manifest.power_on)?;
    let held = match cp.frame {
        0 => [0; 4],
        f => input.frames[f as usize - 1].pads,
    };
    nes.restore(&store.get(&cp.snapshot)?, cp.frame, held)?;
    for f in cp.frame..frame {
        nes.step(input.frames[f as usize])?;
    }
    Ok(nes.wram().to_vec())
}

/// Every frame's hashes for `input`, run with video rendering on, and the
/// first frame whose events differ from `headless` (indexed by frame).
fn run_with_video(
    rom: &[u8],
    input: &InputLog,
    headless: &[Vec<gp_evidence::events::Event>],
) -> Result<(Vec<FrameHash>, Option<u64>), RecordingError> {
    let mut nes = Nes::power_on(rom, PowerOn::default())?;
    nes.set_video(true);
    nes.set_events(true);
    let mut out = Vec::with_capacity(input.frames.len() + 1);
    let hash = |nes: &Nes| -> Result<FrameHash, RecordingError> {
        Ok(FrameHash {
            state: nes.state_hash()?,
            ram: nes.wram_hash(),
        })
    };
    out.push(hash(&nes)?);
    let mut differ_at = None;
    for frame in &input.frames {
        nes.step(*frame)?;
        let events = nes.take_events();
        let f = nes.frame();
        if differ_at.is_none() && headless.get(f as usize).is_none_or(|h| *h != events) {
            differ_at = Some(f);
        }
        out.push(hash(&nes)?);
    }
    Ok((out, differ_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrom;

    fn baseline(digest: &str, lag: usize, equals: u8) -> Baseline {
        Baseline {
            schema: 0,
            source: None,
            sha256: gp_evidence::source::identify_nes(&testrom::build())
                .unwrap()
                .sha256,
            description: None,
            frames: 120,
            holds: vec!["a@10-40".into(), "right+b@50-".into()],
            frame_hashes: digest.into(),
            lag_frames: lag,
            asserts: vec![RamAssert {
                frame: 100,
                label: "BUTTONS".into(),
                addr: testrom::addr::BUTTONS as u16,
                equals,
            }],
        }
    }

    #[test]
    fn video_events_are_compared_frame_by_frame() {
        let rom = testrom::build();
        let input = gp_evidence::input_log::generate(30, &["a@5-20"]).unwrap();
        let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        nes.set_events(true);
        let mut headless = vec![Vec::new()];
        for f in &input.frames {
            nes.step(*f).unwrap();
            headless.push(nes.take_events());
        }
        assert!(headless.iter().any(|e| !e.is_empty()));
        assert_eq!(run_with_video(&rom, &input, &headless).unwrap().1, None);
        headless[17].pop();
        assert_eq!(run_with_video(&rom, &input, &headless).unwrap().1, Some(17));
    }

    #[test]
    fn checks_digest_lag_and_ram() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rom = testrom::build();
        let probe = baseline(&"0".repeat(64), 0, 0);
        let got = run(&store, &probe, "test", &rom, true).unwrap();
        // The test program shifts Right into bit 0 and B into bit 6.
        assert_eq!(got.asserts[0].1, 0b0100_0001);
        assert_eq!(got.video_frame_hashes, Some(got.frame_hashes));
        assert_eq!(got.video_events_differ_at, None);
        assert_eq!(got.failures(&probe).len(), 3, "{:?}", got.failures(&probe));

        let good = baseline(&got.frame_hashes.to_hex(), got.lag_frames, 0b0100_0001);
        let again = run(&store, &good, "test", &rom, false).unwrap();
        assert_eq!(again.failures(&good), Vec::<String>::new());
    }
}
