//! Replaying a stored arcade recording (a gp-arcade run of a romset), as
//! gp-cli's arcade_rec.rs replays it for `gp replay --all`: from power-on,
//! every frame's RAM and state hash and, when the recording has them, its
//! events; then each checkpoint, restored and saved back; then the trace.
//! gp-cli's code is in the binary crate, so the parts replay needs are
//! here too, for the WASM build. A recording from another build of
//! gp-arcade replays as stale, as there.
//!
//! The romset is found by its identity among the zips under the ROM
//! folder, whatever they are named.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gp_arcade::events::{BoardEvent, Happened};
use gp_arcade::{Controls, Romset, Tapper};
use gp_evidence::chunk;
use gp_evidence::events::{self, Event, EventKind, Geometry};
use gp_evidence::frame_hashes::{self, FrameHash};
use gp_evidence::manifest::{RecordingManifest, RuntimeRef};
use gp_evidence::{Hash, InputLog, Store};
use gp_nes::recording::{Checked, Divergence, DivergenceKind};

/// gp-cli's arcade_rec::RECORDING_VERSION: what an arcade recording holds
/// besides the machine. The two must move together.
const RECORDING_VERSION: u32 = 3;

/// The runtime that makes arcade recordings, as gp-cli names it.
fn runtime_ref() -> RuntimeRef {
    RuntimeRef {
        name: "gp-arcade".into(),
        version: format!(
            "{} rec {RECORDING_VERSION}",
            gp_arcade::RUNTIME_VERSION.trim_start_matches("gp-arcade ")
        ),
        commit: None,
    }
}

fn stored(frame: u64, e: &BoardEvent) -> Event {
    Event {
        frame,
        kind: match e.what {
            Happened::PortRead => EventKind::PortRead,
            Happened::SoundLatch => EventKind::SoundLatch,
            Happened::Watchdog => EventKind::Watchdog,
            Happened::AyWrite => EventKind::AyWrite,
            Happened::Irq => EventKind::Irq,
            Happened::SoundIrq => EventKind::SoundIrq,
        },
        scanline: e.line,
        dot: e.dot,
        addr: e.addr,
        value: e.value,
    }
}

fn frame_hash(t: &Tapper) -> FrameHash {
    FrameHash {
        state: Hash::of(&t.recorded_state()),
        ram: Hash::of(&t.ram_record()),
    }
}

fn compare(t: &Tapper, hashes: &[FrameHash], frame: u64) -> Option<DivergenceKind> {
    let want = hashes[frame as usize];
    let have = frame_hash(t);
    if have.ram != want.ram {
        Some(DivergenceKind::Ram)
    } else if have.state != want.state {
        Some(DivergenceKind::State)
    } else {
        None
    }
}

fn controls(f: &gp_evidence::input_log::InputFrame) -> Result<Controls, String> {
    let bits = f.buttons_word();
    if bits > 0xff {
        return Err(format!(
            "buttons {bits:#x}: the arcade board reads bits 0 to 7 only"
        ));
    }
    if f.reset {
        return Err("the arcade board's reset isn't modelled".into());
    }
    Ok(Controls::from_bits(bits))
}

fn load(store: &Store, m: &RecordingManifest) -> Result<(InputLog, Vec<FrameHash>), String> {
    let s = |e: &dyn std::fmt::Display| e.to_string();
    let bytes = store.get(&m.input_log).map_err(|e| s(&e))?;
    let text = String::from_utf8(bytes).map_err(|_| "the recording's input log isn't text")?;
    let input = InputLog::parse(&text).map_err(|e| s(&e))?;
    let id = m.frame_hashes.ok_or("the manifest has no frame hashes")?;
    let hashes = frame_hashes::decode(&store.get(&id).map_err(|e| s(&e))?).map_err(|e| s(&e))?;
    if hashes.len() as u64 != m.frames + 1 || input.frames.len() as u64 != m.frames {
        return Err(format!(
            "recording data is inconsistent: {} frames, {} input frames, {} frame hashes",
            m.frames,
            input.frames.len(),
            hashes.len()
        ));
    }
    Ok((input, hashes))
}

fn events_by_frame(store: &Store, m: &RecordingManifest) -> Result<Vec<Vec<Event>>, String> {
    let stream = m
        .events
        .as_ref()
        .ok_or("the recording has no event stream")?;
    let mut by_frame = Vec::new();
    for (i, id) in stream.chunks.iter().enumerate() {
        let expected = chunk::chunk_range(i, stream.chunk_frames, m.frames);
        let first = expected.start as usize;
        let bytes = store.get(id).map_err(|e| e.to_string())?;
        let c = events::decode_chunk_covering_in(&bytes, expected.clone(), Geometry::MCR)
            .map_err(|e| e.to_string())?;
        by_frame.resize(first + (expected.end - expected.start) as usize, Vec::new());
        for e in c.events {
            by_frame[e.frame as usize].push(e);
        }
    }
    Ok(by_frame)
}

/// Replays from power-on: the frames checked and the first divergence.
fn replay(
    store: &Store,
    m: &RecordingManifest,
    set: &Romset,
) -> Result<(u64, Option<Divergence>), String> {
    let (input, hashes) = load(store, m)?;
    let mut t = Tapper::power_on(set);
    let recorded = match &m.events {
        Some(_) => Some(events_by_frame(store, m)?),
        None => None,
    };
    t.set_events(recorded.is_some());
    if let Some(what) = compare(&t, &hashes, 0) {
        return Ok((0, Some(Divergence { frame: 0, what })));
    }
    let mut checked = 0;
    for f in 0..m.frames {
        t.step(controls(&input.frames[f as usize])?);
        checked += 1;
        let frame = f + 1;
        let happened: Vec<Event> = t.take_events().iter().map(|e| stored(frame, e)).collect();
        let what = compare(&t, &hashes, frame).or_else(|| {
            recorded
                .as_ref()
                .is_some_and(|by| by[frame as usize] != happened)
                .then_some(DivergenceKind::Events)
        });
        if let Some(what) = what {
            return Ok((checked, Some(Divergence { frame, what })));
        }
    }
    Ok((checked, None))
}

fn verify_checkpoints(
    store: &Store,
    m: &RecordingManifest,
    set: &Romset,
) -> Result<Option<Divergence>, String> {
    let (_, hashes) = load(store, m)?;
    let mut t = Tapper::power_on(set);
    for cp in &m.checkpoints {
        let snap = store.get(&cp.snapshot).map_err(|e| e.to_string())?;
        let what = if t.restore(&snap).is_err() || t.snapshot() != snap {
            Some(DivergenceKind::Snapshot)
        } else {
            compare(&t, &hashes, cp.frame)
        };
        if let Some(what) = what {
            return Ok(Some(Divergence {
                frame: cp.frame,
                what,
            }));
        }
    }
    Ok(None)
}

/// An arcade recording checked as `gp replay --all` checks it.
pub fn check(store: &Store, m: &RecordingManifest, set: &Romset) -> Result<Checked, String> {
    let (frames, mut divergence) = replay(store, m, set)?;
    if m.runtime != runtime_ref() {
        return Ok(Checked::Stale {
            frames,
            divergence,
            runtime: gp_nes::recording::describe_runtime(&m.runtime),
        });
    }
    if divergence.is_none() {
        divergence = verify_checkpoints(store, m, set)?;
    }
    if divergence.is_none() && m.trace.is_some() {
        divergence = gp_nes::recording::verify_trace(store, m).map_err(|e| e.to_string())?;
    }
    Ok(Checked::Replayed {
        frames,
        divergence,
        recorded_by: None,
    })
}

/// Every romset zip under `dir`, by its identity.
pub fn index_romsets(dir: &Path) -> HashMap<String, PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) {
                out.push(p);
            }
        }
    }
    let mut zips = Vec::new();
    walk(dir, &mut zips);
    zips.sort();
    let mut out = HashMap::new();
    for z in zips {
        if let Ok(set) = Romset::load_zip(&z) {
            out.entry(set.identity).or_insert(z);
        }
    }
    out
}
