//! Making and replaying recordings on the NES runtime.
//!
//! A recording runs a ROM from power-on through an input log and stores what
//! replay needs: the input log as the game actually polled it, every frame's
//! state hashes, checkpoints, and a manifest naming them all. Replay runs the
//! same inputs again and compares every frame.

use std::time::Instant;

use gp_evidence::chunk::{self, ChunkError};
use gp_evidence::events::{self, Event, EventWriter};
use gp_evidence::frame_hashes::{self, FrameHash};
use gp_evidence::manifest::{
    Checkpoint, EventsRef, PowerOn, Provenance, RECORDING_SCHEMA, RecordingManifest, RuntimeRef,
    SourceRef, TraceRef,
};
use gp_evidence::recordings::{self, IndexEntry};
use gp_evidence::trace::{self, FieldTable, TraceWriter};
use gp_evidence::{Hash, InputLog, Store};

use crate::{Nes, NesError, canonical, runtime_ref};

#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error(transparent)]
    Nes(#[from] NesError),
    #[error(transparent)]
    Store(#[from] gp_evidence::store::StoreError),
    #[error(transparent)]
    Recordings(#[from] recordings::RecordingsError),
    #[error("the input log is empty")]
    EmptyInput,
    #[error("the input log has {0} controllers; gp-nes runs ports 1 and 2 only")]
    TooManyControllers(u8),
    #[error("the checkpoint interval must be at least 1 frame")]
    ZeroInterval,
    #[error("trace and event chunks must hold at least 1 frame")]
    ZeroChunk,
    #[error("no checkpoint at frame {frame}; checkpoints are at frames {available}")]
    NoCheckpoint { frame: u64, available: String },
    #[error(
        "the recording's frame hashes are format {0}, which hashed tetanes-core's save state at scanline 240; this runtime checks format 1, a canonical state at the start of vblank. Record it again"
    )]
    OldFrameHashes(u32),
    #[error("recording data is inconsistent: {0}")]
    Corrupt(String),
    #[error(transparent)]
    Chunk(#[from] ChunkError),
    #[error("the recording has no frame trace")]
    NoTrace,
    #[error(
        "the stored trace's state at frame {0} doesn't hash to the recorded frame hash, so the trace is damaged; `gp trace verify` checks all of it"
    )]
    TraceMismatch(u64),
    #[error("frame {frame} is past the recording's last frame, {frames}")]
    PastEnd { frame: u64, frames: u64 },
    #[error(
        "this recording was made by {recorded}, and this is {current}. Its checkpoints are snapshots only the runtime that made them can restore. Replay it from power-on, or re-record it"
    )]
    OtherRuntime { recorded: String, current: String },
    #[error(
        "this recording was made by {recorded}, and this is {current}, which differs only in its fork patch, but the checkpoint at frame {frame} doesn't round-trip under it. Replay it from power-on, or re-record it"
    )]
    CheckpointDoesNotRoundTrip {
        frame: u64,
        recorded: String,
        current: String,
    },
}

/// What to record. The run itself goes in the manifest and decides the ID;
/// `source_id`, `name`, `created_at` and `notes` go only in the index entry.
pub struct RecordRequest<'a> {
    pub rom: &'a [u8],
    pub source: SourceRef,
    /// The registry ID, for the index.
    pub source_id: Option<String>,
    pub power_on: PowerOn,
    pub input: &'a InputLog,
    pub checkpoint_interval: u32,
    pub provenance: Provenance,
    pub name: String,
    pub created_at: String,
    pub notes: Option<String>,
}

/// What a recording keeps besides its input log, frame hashes and
/// checkpoints. The default keeps neither, which is what [`record`] does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capture {
    /// A frame trace, in chunks of this many frames.
    pub trace: Option<u32>,
    /// The event stream, in chunks of this many frames.
    pub events: Option<u32>,
}

impl Capture {
    /// The trace and the event stream, in chunks of
    /// [`chunk::DEFAULT_CHUNK_FRAMES`].
    pub const ALL: Self = Self {
        trace: Some(chunk::DEFAULT_CHUNK_FRAMES),
        events: Some(chunk::DEFAULT_CHUNK_FRAMES),
    };
}

#[derive(Debug)]
pub struct Recorded {
    pub id: Hash,
    pub manifest: RecordingManifest,
    pub frames_per_second: f64,
    /// Bytes of all the recording's objects.
    pub bytes: u64,
    /// Bytes of objects the store didn't already have.
    pub new_bytes: u64,
    /// Bytes of the frame trace's chunks and field table; zero without a trace.
    pub trace_bytes: u64,
    /// Bytes of the event stream's chunks; zero without one.
    pub events_bytes: u64,
    pub lag_frames: usize,
    /// Whether the store already had this recording: the same run, made
    /// before under any name.
    pub existing: bool,
}

fn check_input(input: &InputLog) -> Result<(), RecordingError> {
    if input.frames.is_empty() {
        return Err(RecordingError::EmptyInput);
    }
    if input.controllers > 2 {
        return Err(RecordingError::TooManyControllers(input.controllers));
    }
    Ok(())
}

/// Records without a trace or event stream.
pub fn record(store: &Store, req: RecordRequest<'_>) -> Result<Recorded, RecordingError> {
    record_with(store, req, Capture::default())
}

pub fn record_with(
    store: &Store,
    req: RecordRequest<'_>,
    capture: Capture,
) -> Result<Recorded, RecordingError> {
    check_input(req.input)?;
    if capture.trace == Some(0) || capture.events == Some(0) {
        return Err(RecordingError::ZeroChunk);
    }
    if req.checkpoint_interval == 0 {
        return Err(RecordingError::ZeroInterval);
    }
    let interval = u64::from(req.checkpoint_interval);
    let mut nes = Nes::power_on(req.rom, req.power_on)?;
    let mut observed = req.input.clone();
    let mut hashes = Vec::with_capacity(req.input.frames.len() + 1);
    let mut checkpoints = Vec::new();
    let (mut bytes, mut new_bytes) = (0u64, 0u64);
    let mut put = |data: &[u8]| -> Result<Hash, RecordingError> {
        bytes += data.len() as u64;
        if !store.contains(&Hash::of(data)) {
            new_bytes += data.len() as u64;
        }
        Ok(store.put_bytes(data)?)
    };
    let (mut trace_bytes, mut events_bytes) = (0u64, 0u64);
    let mut trace = match capture.trace {
        Some(chunk_frames) => {
            let table = canonical::field_table().encode();
            trace_bytes += table.len() as u64;
            let fields = put(&table)?;
            Some((TraceWriter::new(chunk_frames, fields), fields, Vec::new()))
        }
        None => None,
    };
    let mut event_stream = capture
        .events
        .map(|chunk_frames| (EventWriter::new(chunk_frames), Vec::new()));
    nes.set_events(event_stream.is_some());

    let start = Instant::now();
    for i in 0..=observed.frames.len() {
        if i > 0 {
            let frame = &mut observed.frames[i - 1];
            frame.polled = nes.step(*frame)?.polled;
        }
        let state = nes.canonical_state()?;
        hashes.push(FrameHash {
            state: Hash::of(&state),
            ram: nes.wram_hash(),
        });
        if let Some((writer, _, chunks)) = &mut trace
            && let Some(c) = writer.push(&state)?
        {
            trace_bytes += c.len() as u64;
            chunks.push(put(&c)?);
        }
        if let Some((writer, chunks)) = &mut event_stream
            && let Some(c) = writer.push(&nes.take_events())
        {
            events_bytes += c.len() as u64;
            chunks.push(put(&c)?);
        }
        let n = i as u64;
        if n.is_multiple_of(interval) {
            checkpoints.push(Checkpoint {
                frame: n,
                snapshot: put(&nes.snapshot()?)?,
            });
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let trace = match trace {
        Some((writer, fields, mut chunks)) => {
            if let Some(c) = writer.finish() {
                trace_bytes += c.len() as u64;
                chunks.push(put(&c)?);
            }
            Some(TraceRef {
                schema: trace::SCHEMA,
                chunk_frames: capture.trace.expect("trace is on"),
                fields,
                chunks,
            })
        }
        None => None,
    };
    let events = match event_stream {
        Some((writer, mut chunks)) => {
            if let Some(c) = writer.finish() {
                events_bytes += c.len() as u64;
                chunks.push(put(&c)?);
            }
            Some(EventsRef {
                schema: events::SCHEMA,
                chunk_frames: capture.events.expect("events are on"),
                chunks,
            })
        }
        None => None,
    };

    let manifest = RecordingManifest {
        schema: RECORDING_SCHEMA,
        source: req.source,
        runtime: runtime_ref(),
        power_on: req.power_on,
        frames: observed.frames.len() as u64,
        input_log: put(observed.to_text().as_bytes())?,
        checkpoint_interval: req.checkpoint_interval,
        frame_hashes: Some(put(&frame_hashes::encode(&hashes))?),
        checkpoints,
        trace,
        events,
        provenance: req.provenance,
    };
    manifest
        .validate()
        .map_err(|e| RecordingError::Corrupt(e.to_string()))?;
    let json = manifest.to_json();
    let existing = store.contains(&Hash::of(json.as_bytes()));
    let id = put(json.as_bytes())?;
    recordings::append(
        store,
        &IndexEntry {
            id,
            created_at: req.created_at,
            source: req.source_id,
            frames: manifest.frames,
            name: req.name,
            notes: req.notes,
            golden: false,
        },
    )?;
    Ok(Recorded {
        id,
        lag_frames: observed.lag_frames(),
        frames_per_second: observed.frames.len() as f64 / elapsed.max(1e-9),
        manifest,
        bytes,
        new_bytes,
        trace_bytes,
        events_bytes,
        existing,
    })
}

/// Where a replay first disagreed with its recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    pub frame: u64,
    pub what: DivergenceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceKind {
    /// Internal RAM differs: game logic took another path.
    Ram,
    /// RAM matches but the rest of the console's state doesn't.
    State,
    /// The game polled the controller on a frame recorded as a lag frame, or the reverse.
    Polled,
    /// RAM, state and polling match, but the events within the frame differ.
    Events,
    /// The stored frame trace doesn't match the frame hashes.
    Trace,
    /// A checkpoint's snapshot doesn't round-trip: this runtime can't load
    /// it, or saving the restored console gives other bytes.
    Snapshot,
}

impl DivergenceKind {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Ram => "internal RAM differs (game logic diverged)",
            Self::State => "RAM matches, but CPU, PPU, APU, mapper or controller state differs",
            Self::Polled => "the game's controller polling differs from the recording",
            Self::Events => "state matches, but the events within the frame differ",
            Self::Trace => "the stored frame trace doesn't match the frame hashes",
            Self::Snapshot => {
                "a checkpoint's snapshot doesn't round-trip (restoring it and saving again gives other bytes)"
            }
        }
    }
}

#[derive(Debug)]
pub struct ReplayReport {
    pub from_frame: u64,
    pub frames_checked: u64,
    pub frames_per_second: f64,
    pub divergence: Option<Divergence>,
}

/// Whether this build of gp-nes is the runtime that made the recording. A
/// recording from another runtime can still be replayed from power-on, but
/// its checkpoints restore only under [`Lineage::PatchOnly`], and a
/// divergence means re-record rather than an emulation bug.
pub fn made_by_this_runtime(manifest: &RecordingManifest) -> bool {
    manifest.runtime == runtime_ref()
}

/// How this runtime relates to the one that made a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lineage {
    /// This runtime made it.
    Same,
    /// Another build of gp-nes with the same [`crate::BEHAVIOUR_VERSION`] and
    /// the same upstream `tetanes-core`, whose fork patch differs: a change
    /// that claims to only observe (an event or coverage hook, say). Its
    /// checkpoints restore where each one round-trips
    /// ([`restore_checked`]); replay checks the claim frame by frame.
    PatchOnly,
    /// Another behaviour version, upstream release or runtime. Its
    /// checkpoints are never restored.
    Other,
}

/// The runtime version without its patch hash: `2 (tetanes-core 0.17.0`
/// from `2 (tetanes-core 0.17.0 + patch 0123456789ab)`.
fn without_patch(version: &str) -> Option<&str> {
    version.split_once(" + patch ").map(|(head, _)| head)
}

/// The recording's [`Lineage`], from its runtime identity alone.
pub fn lineage(manifest: &RecordingManifest) -> Lineage {
    lineage_of(&manifest.runtime)
}

/// The [`Lineage`] of a recording made by `theirs`.
pub fn lineage_of(theirs: &RuntimeRef) -> Lineage {
    let ours = runtime_ref();
    if *theirs == ours {
        Lineage::Same
    } else if theirs.name == ours.name
        && theirs.commit == ours.commit
        && without_patch(&theirs.version).is_some()
        && without_patch(&theirs.version) == without_patch(&ours.version)
    {
        Lineage::PatchOnly
    } else {
        Lineage::Other
    }
}

/// Restores checkpoint `cp` into `nes` and checks that it round-trips: the
/// restored console hashes to the frame hashes recorded for its frame, and
/// saving it again gives the stored snapshot byte for byte. Returns how it
/// failed, or `None`. A snapshot this runtime can't load at all fails as
/// [`DivergenceKind::Snapshot`]; `nes` is then left as it was.
fn restore_checked(
    nes: &mut Nes,
    store: &Store,
    cp: &gp_evidence::manifest::Checkpoint,
    held: [u8; 4],
    hashes: &[FrameHash],
) -> Result<Option<DivergenceKind>, RecordingError> {
    let snapshot = store.get(&cp.snapshot)?;
    if nes.restore(&snapshot, cp.frame, held).is_err() {
        return Ok(Some(DivergenceKind::Snapshot));
    }
    if let Some(what) = compare(nes, hashes, cp.frame)? {
        return Ok(Some(what));
    }
    if nes.snapshot()? != snapshot {
        return Ok(Some(DivergenceKind::Snapshot));
    }
    Ok(None)
}

/// Restores checkpoint `cp` if this runtime may use it: always under
/// [`Lineage::Same`], when it round-trips under [`Lineage::PatchOnly`], and
/// never under [`Lineage::Other`]. Returns whether it did; when it didn't,
/// `nes` is powered on afresh, so the caller replays from frame 0.
pub(crate) fn restore_if_usable(
    nes: &mut Nes,
    rom: &[u8],
    store: &Store,
    manifest: &RecordingManifest,
    cp: &gp_evidence::manifest::Checkpoint,
    held: [u8; 4],
    hashes: &[FrameHash],
) -> Result<bool, RecordingError> {
    match lineage(manifest) {
        Lineage::Same => {
            nes.restore(&store.get(&cp.snapshot)?, cp.frame, held)?;
            Ok(true)
        }
        Lineage::PatchOnly => {
            if restore_checked(nes, store, cp, held, hashes)?.is_none() {
                return Ok(true);
            }
            let events = nes.events;
            *nes = Nes::power_on(rom, manifest.power_on)?;
            nes.set_events(events);
            Ok(false)
        }
        Lineage::Other => Ok(false),
    }
}

/// Whether this runtime may restore the recording's checkpoints: when it
/// made the recording, or under [`Lineage::PatchOnly`] when every checkpoint
/// round-trips ([`verify_checkpoints`] finds nothing).
pub fn keeps_checkpoints(
    store: &Store,
    manifest: &RecordingManifest,
    rom: &[u8],
) -> Result<bool, RecordingError> {
    Ok(match lineage(manifest) {
        Lineage::Same => true,
        Lineage::PatchOnly => verify_checkpoints(store, manifest, rom)?.is_none(),
        Lineage::Other => false,
    })
}

/// A runtime as messages name it, such as `gp-nes 2 (tetanes-core 0.17.0 + patch <12 hex digits>)`.
pub fn describe_runtime(runtime: &RuntimeRef) -> String {
    format!("{} {}", runtime.name, runtime.version)
}

/// Refuses a recording whose checkpoints this runtime never restores
/// ([`Lineage::Other`]).
fn checkpoints_allowed(manifest: &RecordingManifest) -> Result<(), RecordingError> {
    if lineage(manifest) != Lineage::Other {
        return Ok(());
    }
    Err(RecordingError::OtherRuntime {
        recorded: describe_runtime(&manifest.runtime),
        current: describe_runtime(&runtime_ref()),
    })
}

/// Replays a recording from power-on, or from the checkpoint at
/// `from_checkpoint`, and compares every frame with what was recorded: its
/// hashes, whether it polled the controller, and its events when the
/// recording has an event stream. Starting from a checkpoint needs the
/// runtime that made the recording.
pub fn replay(
    store: &Store,
    manifest: &RecordingManifest,
    rom: &[u8],
    from_checkpoint: Option<u64>,
) -> Result<ReplayReport, RecordingError> {
    let (input, hashes) = load(store, manifest)?;
    let mut nes = Nes::power_on(rom, manifest.power_on)?;
    let recorded_events = match &manifest.events {
        Some(_) => Some(events_by_frame(store, manifest)?),
        None => None,
    };
    nes.set_events(recorded_events.is_some());
    let from = match from_checkpoint {
        None | Some(0) => 0,
        Some(f) => {
            checkpoints_allowed(manifest)?;
            let cp = manifest
                .checkpoints
                .iter()
                .find(|c| c.frame == f)
                .ok_or_else(|| RecordingError::NoCheckpoint {
                    frame: f,
                    available: checkpoint_frames(manifest),
                })?;
            let held = input.frames[f as usize - 1].pads;
            if !restore_if_usable(&mut nes, rom, store, manifest, cp, held, &hashes)? {
                return Err(RecordingError::CheckpointDoesNotRoundTrip {
                    frame: f,
                    recorded: describe_runtime(&manifest.runtime),
                    current: describe_runtime(&runtime_ref()),
                });
            }
            f
        }
    };
    let compare = |nes: &Nes, frame: u64| compare(nes, &hashes, frame);

    let start = Instant::now();
    if let Some(what) = compare(&nes, from)? {
        return Ok(ReplayReport {
            from_frame: from,
            frames_checked: 0,
            frames_per_second: 0.0,
            divergence: Some(Divergence { frame: from, what }),
        });
    }
    let mut checked = 0;
    for f in from..manifest.frames {
        let recorded = input.frames[f as usize];
        let polled = nes.step(recorded)?.polled;
        checked += 1;
        let frame = f + 1;
        let happened = nes.take_events();
        let what = if polled != recorded.polled {
            Some(DivergenceKind::Polled)
        } else if let Some(what) = compare(&nes, frame)? {
            Some(what)
        } else if recorded_events
            .as_ref()
            .is_some_and(|by_frame| by_frame[frame as usize] != happened)
        {
            Some(DivergenceKind::Events)
        } else {
            None
        };
        if let Some(what) = what {
            return Ok(ReplayReport {
                from_frame: from,
                frames_checked: checked,
                frames_per_second: checked as f64 / start.elapsed().as_secs_f64().max(1e-9),
                divergence: Some(Divergence { frame, what }),
            });
        }
    }
    Ok(ReplayReport {
        from_frame: from,
        frames_checked: checked,
        frames_per_second: checked as f64 / start.elapsed().as_secs_f64().max(1e-9),
        divergence: None,
    })
}

pub(crate) fn load(
    store: &Store,
    manifest: &RecordingManifest,
) -> Result<(InputLog, Vec<FrameHash>), RecordingError> {
    let input = InputLog::parse(
        std::str::from_utf8(&store.get(&manifest.input_log)?)
            .map_err(|_| RecordingError::Corrupt("input log isn't text".into()))?,
    )
    .map_err(|e| RecordingError::Corrupt(e.to_string()))?;
    check_input(&input)?;
    let hashes_id = manifest
        .frame_hashes
        .ok_or_else(|| RecordingError::Corrupt("the manifest has no frame hashes".into()))?;
    let bytes = store.get(&hashes_id)?;
    let version =
        frame_hashes::version(&bytes).map_err(|e| RecordingError::Corrupt(e.to_string()))?;
    if version != frame_hashes::FORMAT_VERSION {
        return Err(RecordingError::OldFrameHashes(version));
    }
    let hashes =
        frame_hashes::decode(&bytes).map_err(|e| RecordingError::Corrupt(e.to_string()))?;
    if hashes.len() as u64 != manifest.frames + 1 || input.frames.len() as u64 != manifest.frames {
        return Err(RecordingError::Corrupt(format!(
            "{} frames, {} input frames, {} frame hashes",
            manifest.frames,
            input.frames.len(),
            hashes.len()
        )));
    }
    Ok((input, hashes))
}

fn compare(
    nes: &Nes,
    hashes: &[FrameHash],
    frame: u64,
) -> Result<Option<DivergenceKind>, RecordingError> {
    let expected = hashes[frame as usize];
    if nes.wram_hash() != expected.ram {
        return Ok(Some(DivergenceKind::Ram));
    }
    if nes.state_hash()? != expected.state {
        return Ok(Some(DivergenceKind::State));
    }
    Ok(None)
}

fn checkpoint_frames(manifest: &RecordingManifest) -> String {
    manifest
        .checkpoints
        .iter()
        .map(|c| c.frame.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Restores every checkpoint and checks it against the frame hashes
/// recorded for its frame. Replay from power-on never restores, so this is
/// what catches a snapshot that no longer loads into the state it was taken
/// from (a changed mapper or state layout, say). Each must also save back to
/// the same bytes. Returns the first mismatch. Refuses a recording whose
/// checkpoints this runtime never restores ([`Lineage::Other`]).
pub fn verify_checkpoints(
    store: &Store,
    manifest: &RecordingManifest,
    rom: &[u8],
) -> Result<Option<Divergence>, RecordingError> {
    checkpoints_allowed(manifest)?;
    let (input, hashes) = load(store, manifest)?;
    let mut nes = Nes::power_on(rom, manifest.power_on)?;
    for cp in &manifest.checkpoints {
        let held = match cp.frame {
            0 => [0; 4],
            f => input.frames[f as usize - 1].pads,
        };
        if let Some(what) = restore_checked(&mut nes, store, cp, held, &hashes)? {
            return Ok(Some(Divergence {
                frame: cp.frame,
                what,
            }));
        }
    }
    Ok(None)
}

/// Every frame's recorded events, indexed by frame (0 to the last).
pub fn events_by_frame(
    store: &Store,
    manifest: &RecordingManifest,
) -> Result<Vec<Vec<Event>>, RecordingError> {
    let stream = manifest
        .events
        .as_ref()
        .ok_or_else(|| RecordingError::Corrupt("the recording has no event stream".into()))?;
    // Grown chunk by chunk, so a manifest claiming more frames than its
    // chunks hold can't allocate for them.
    let mut by_frame = Vec::new();
    for (i, id) in stream.chunks.iter().enumerate() {
        let expected = chunk::chunk_range(i, stream.chunk_frames, manifest.frames);
        let first = expected.start as usize;
        let c = events::decode_chunk_covering(&store.get(id)?, expected.clone())?;
        by_frame.resize(first + (expected.end - expected.start) as usize, Vec::new());
        for e in c.events {
            by_frame[e.frame as usize].push(e);
        }
    }
    Ok(by_frame)
}

/// The recorded events of frames in `range` (clamped to the recording), in
/// order, or `None` if the recording has no event stream. Decodes only the
/// chunks the range touches. The events are served as stored: nothing here
/// can check them without the runtime, so replay is what checks them.
pub fn stored_events(
    store: &Store,
    manifest: &RecordingManifest,
    range: std::ops::Range<u64>,
) -> Result<Option<Vec<Event>>, RecordingError> {
    let Some(stream) = &manifest.events else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for (i, id) in stream.chunks.iter().enumerate() {
        let covers = chunk::chunk_range(i, stream.chunk_frames, manifest.frames);
        if covers.end <= range.start || covers.start >= range.end {
            continue;
        }
        let c = events::decode_chunk_covering(&store.get(id)?, covers)?;
        out.extend(c.events.into_iter().filter(|e| range.contains(&e.frame)));
    }
    Ok(Some(out))
}

/// The field table a recording's states follow: its trace's, or this
/// runtime's for a recording without one (its states come from replay).
pub fn field_table(
    store: &Store,
    manifest: &RecordingManifest,
) -> Result<FieldTable, RecordingError> {
    match &manifest.trace {
        Some(t) => Ok(FieldTable::decode(&store.get(&t.fields)?)?),
        None => Ok(canonical::field_table()),
    }
}

/// The state at `frame` from the recording's trace, or `None` if it has no
/// trace. The state is hashed and checked against the recorded frame hash
/// (about 10 us), so a damaged trace is an error rather than a wrong answer.
pub fn trace_state(
    store: &Store,
    manifest: &RecordingManifest,
    frame: u64,
) -> Result<Option<Vec<u8>>, RecordingError> {
    let Some(t) = &manifest.trace else {
        return Ok(None);
    };
    if frame > manifest.frames {
        return Err(RecordingError::PastEnd {
            frame,
            frames: manifest.frames,
        });
    }
    let i = (frame / u64::from(t.chunk_frames)) as usize;
    let id = t.chunks.get(i).ok_or_else(|| {
        RecordingError::Corrupt(format!("the trace has no chunk {i}, for frame {frame}"))
    })?;
    let expected = chunk::chunk_range(i, t.chunk_frames, manifest.frames);
    let (_, hashes) = load(store, manifest)?;
    let table = FieldTable::decode(&store.get(&t.fields)?)?;
    let state = trace::decode_state(&store.get(id)?, expected, &table, frame)?;
    if Hash::of(&state) != hashes[frame as usize].state {
        return Err(RecordingError::TraceMismatch(frame));
    }
    Ok(Some(state))
}

/// Calls `each` with the state of every frame in `frames` from the
/// recording's trace, in order, until it returns false. Each state is
/// checked against its frame hash first, as [`trace_state`] checks one.
/// Decodes only the chunks the frames touch, and keeps one state at a
/// time. Returns false, having called nothing, if the recording has no
/// trace.
pub fn scan_trace(
    store: &Store,
    manifest: &RecordingManifest,
    frames: std::ops::RangeInclusive<u64>,
    mut each: impl FnMut(u64, &[u8]) -> bool,
) -> Result<bool, RecordingError> {
    let Some(t) = &manifest.trace else {
        return Ok(false);
    };
    if *frames.end() > manifest.frames {
        return Err(RecordingError::PastEnd {
            frame: *frames.end(),
            frames: manifest.frames,
        });
    }
    let (_, hashes) = load(store, manifest)?;
    let table = FieldTable::decode(&store.get(&t.fields)?)?;
    let mut going = true;
    let mut mismatch = None;
    for (i, id) in t.chunks.iter().enumerate() {
        let covers = chunk::chunk_range(i, t.chunk_frames, manifest.frames);
        if !going || covers.end <= *frames.start() || covers.start > *frames.end() {
            continue;
        }
        trace::decode_each(&store.get(id)?, &covers, &table, |frame, state| {
            if !going || !frames.contains(&frame) {
                return;
            }
            if Hash::of(state) != hashes[frame as usize].state {
                mismatch = Some(frame);
                going = false;
                return;
            }
            going = each(frame, state);
        })?;
        if let Some(frame) = mismatch {
            return Err(RecordingError::TraceMismatch(frame));
        }
    }
    Ok(true)
}

/// A console at `frame` of the recording, by replay: from the nearest
/// checkpoint at or before `start_by` if this runtime made the recording,
/// otherwise from power-on. Calls `on_frame` after each frame it runs. With
/// `events` on, the event log is on throughout.
fn replay_to(
    store: &Store,
    manifest: &RecordingManifest,
    rom: &[u8],
    start_by: u64,
    frame: u64,
    mut on_frame: impl FnMut(&mut Nes) -> Result<(), RecordingError>,
    events: bool,
) -> Result<Nes, RecordingError> {
    if frame > manifest.frames {
        return Err(RecordingError::PastEnd {
            frame,
            frames: manifest.frames,
        });
    }
    let (input, hashes) = load(store, manifest)?;
    let mut nes = Nes::power_on(rom, manifest.power_on)?;
    nes.set_events(events);
    let checkpoint = manifest
        .checkpoints
        .iter()
        .rev()
        .find(|c| c.frame > 0 && c.frame <= start_by.min(frame));
    if let Some(cp) = checkpoint {
        let held = input.frames[cp.frame as usize - 1].pads;
        restore_if_usable(&mut nes, rom, store, manifest, cp, held, &hashes)?;
    }
    while nes.frame() < frame {
        nes.step(input.frames[nes.frame() as usize])?;
        on_frame(&mut nes)?;
    }
    Ok(nes)
}

/// The canonical state at `frame`, by replay (see [`replay_to`]). For a
/// recording without a trace.
pub fn replayed_state(
    store: &Store,
    manifest: &RecordingManifest,
    rom: &[u8],
    frame: u64,
) -> Result<Vec<u8>, RecordingError> {
    Ok(replay_to(store, manifest, rom, frame, frame, |_| Ok(()), false)?.canonical_state()?)
}

/// The events of frames in `range`, by replay. For a recording without an
/// event stream.
pub fn replayed_events(
    store: &Store,
    manifest: &RecordingManifest,
    rom: &[u8],
    range: std::ops::Range<u64>,
) -> Result<Vec<Event>, RecordingError> {
    let end = range.end.min(manifest.frames + 1);
    let mut out = Vec::new();
    if range.start >= end {
        return Ok(out);
    }
    // Frame N's events happen while running from N-1 to N.
    replay_to(
        store,
        manifest,
        rom,
        range.start.saturating_sub(1),
        end - 1,
        |nes| {
            let events = nes.take_events();
            if range.contains(&nes.frame()) {
                out.extend(events);
            }
            Ok(())
        },
        true,
    )?;
    Ok(out)
}

/// Checks a recording's trace against its frame hashes: every chunk covers
/// the frames it should, follows the field table the manifest names, and
/// every frame's state hashes to the recorded state hash. Returns the first
/// frame that doesn't. Needs no ROM and no runtime, so it checks traces made
/// by any runtime.
pub fn verify_trace(
    store: &Store,
    manifest: &RecordingManifest,
) -> Result<Option<Divergence>, RecordingError> {
    let t = manifest.trace.as_ref().ok_or(RecordingError::NoTrace)?;
    let (_, hashes) = load(store, manifest)?;
    let table = FieldTable::decode(&store.get(&t.fields)?)?;
    for (i, id) in t.chunks.iter().enumerate() {
        let expected = chunk::chunk_range(i, t.chunk_frames, manifest.frames);
        // Only the first mismatch is kept, so no chunk's states pile up.
        let mut first_bad = None;
        trace::decode_each(&store.get(id)?, &expected, &table, |frame, state| {
            if first_bad.is_none() && Hash::of(state) != hashes[frame as usize].state {
                first_bad = Some(frame);
            }
        })?;
        if let Some(frame) = first_bad {
            return Ok(Some(Divergence {
                frame,
                what: DivergenceKind::Trace,
            }));
        }
    }
    Ok(None)
}

/// A recording's frame hashes, frames 0 to the last, if they're the format
/// this runtime writes.
pub fn frame_hashes_of(
    store: &Store,
    manifest: &RecordingManifest,
) -> Result<Vec<FrameHash>, RecordingError> {
    Ok(load(store, manifest)?.1)
}

/// What checking one stored recording found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    /// Made by this runtime, or by one that differs only in its fork patch
    /// ([`Lineage::PatchOnly`], named in `recorded_by`) whose checkpoints
    /// all round-trip: frames replayed from power-on, then every checkpoint
    /// restored. `divergence` is the first frame that differed.
    Replayed {
        frames: u64,
        divergence: Option<Divergence>,
        recorded_by: Option<String>,
    },
    /// Made by another runtime, so replayed from power-on only. A
    /// `divergence` of kind [`DivergenceKind::Trace`] is a damaged trace,
    /// which fails the check like any divergence.
    Stale {
        frames: u64,
        divergence: Option<Divergence>,
        runtime: String,
    },
    /// Made by another runtime whose frame hashes this one can't compare
    /// (an older frame hash format), so not replayed at all.
    StaleFormat { runtime: String, version: u32 },
    /// Made by a runtime other than `gp-nes` altogether, such as Mesen
    /// (`gp oracle record`). Its states follow another field table, so
    /// replaying it here would differ from frame 0 and say nothing. Only its
    /// trace is checked against its frame hashes, which needs no runtime.
    OtherRuntime {
        runtime: String,
        divergence: Option<Divergence>,
    },
}

impl Checked {
    /// `ok`, `diverged`, `stale` or `other`.
    pub fn result(&self) -> &'static str {
        match self {
            Self::Replayed {
                divergence: None, ..
            } => "ok",
            Self::Replayed { .. } => "diverged",
            Self::Stale {
                divergence: Some(d),
                ..
            } if d.what == DivergenceKind::Trace => "diverged",
            Self::Stale { .. } | Self::StaleFormat { .. } => "stale",
            Self::OtherRuntime {
                divergence: Some(_),
                ..
            } => "diverged",
            Self::OtherRuntime { .. } => "other",
        }
    }

    /// One report line, as `gp replay --all` prints it.
    pub fn line(&self, short_id: &str, name: &str) -> String {
        match self {
            Self::Replayed {
                frames,
                divergence: None,
                recorded_by,
            } => format!(
                "ok       {short_id}  {name}  {frames} frames, checkpoints restore{}",
                patch_only_note(recorded_by)
            ),
            Self::Replayed {
                divergence: Some(d),
                recorded_by,
                ..
            } => format!(
                "DIVERGED {short_id}  {name}  at frame {}: {}{}",
                d.frame,
                d.what.describe(),
                patch_only_note(recorded_by)
            ),
            Self::Stale {
                frames,
                divergence: None,
                runtime,
            } => format!(
                "STALE    {short_id}  {name}  {frames} frames match from power-on, checkpoints not used (recorded by {runtime})"
            ),
            Self::Stale {
                divergence: Some(d),
                runtime,
                ..
            } if d.what == DivergenceKind::Trace => format!(
                "DIVERGED {short_id}  {name}  at frame {}: {} (recorded by {runtime}, but the trace needs no runtime to check)",
                d.frame,
                d.what.describe()
            ),
            Self::Stale {
                divergence: Some(d),
                runtime,
                ..
            } => format!(
                "STALE    {short_id}  {name}  differs from frame {}: {} (recorded by {runtime})",
                d.frame,
                d.what.describe()
            ),
            Self::StaleFormat { runtime, version } => format!(
                "STALE    {short_id}  {name}  not replayed: its frame hashes are format {version}, which this runtime can't compare (recorded by {runtime})"
            ),
            Self::OtherRuntime {
                runtime,
                divergence: None,
            } => format!(
                "other    {short_id}  {name}  not replayed: recorded by {runtime}, which gp-nes doesn't replay; its trace matches its frame hashes"
            ),
            Self::OtherRuntime {
                runtime,
                divergence: Some(d),
            } => format!(
                "DIVERGED {short_id}  {name}  at frame {}: {} (recorded by {runtime}, but the trace needs no runtime to check)",
                d.frame,
                d.what.describe()
            ),
        }
    }
}

fn patch_only_note(recorded_by: &Option<String>) -> String {
    recorded_by.as_ref().map_or_else(String::new, |r| {
        format!(" (recorded by {r}, which differs only in its fork patch)")
    })
}

/// Checks one stored recording the way `gp replay --all` does: replays it
/// from power-on and, if it was made by this runtime and matched, restores
/// every checkpoint, then checks its trace, if it has one. A recording from
/// another runtime is stale: replayed from power-on only, or not at all when
/// its frame hashes are an older format. Its trace needs no runtime, so it's
/// checked all the same, and a trace that doesn't match is damage rather
/// than staleness. A recording made by a runtime other than `gp-nes`, such
/// as Mesen, is never replayed; only its trace is checked.
///
/// A recording from a runtime that differs only in its fork patch
/// ([`Lineage::PatchOnly`]) is checked like one of ours when its
/// checkpoints all round-trip, so a divergence in RAM, state or polling is
/// a divergence: the patch changed behaviour without a
/// [`crate::BEHAVIOUR_VERSION`] bump. When a checkpoint doesn't round-trip,
/// or only the events differ (a patch may add an event hook), it's stale.
pub fn check(
    store: &Store,
    manifest: &RecordingManifest,
    rom: &[u8],
) -> Result<Checked, RecordingError> {
    if manifest.runtime.name != runtime_ref().name {
        return Ok(Checked::OtherRuntime {
            runtime: describe_runtime(&manifest.runtime),
            divergence: match manifest.trace {
                Some(_) => verify_trace(store, manifest)?,
                None => None,
            },
        });
    }
    let lineage = lineage(manifest);
    let mut ours = lineage != Lineage::Other;
    let report = match replay(store, manifest, rom, None) {
        Err(RecordingError::OldFrameHashes(version)) if !ours => {
            return Ok(Checked::StaleFormat {
                runtime: describe_runtime(&manifest.runtime),
                version,
            });
        }
        r => r?,
    };
    let recorded_by = (lineage == Lineage::PatchOnly).then(|| describe_runtime(&manifest.runtime));
    let mut divergence = report.divergence;
    if lineage == Lineage::PatchOnly {
        // A patch-only runtime keeps checkpoints only when they all
        // round-trip, whatever the replay found; and a patch may add or
        // change events without changing a hash, so differing events alone
        // make the recording stale rather than diverged.
        if verify_checkpoints(store, manifest, rom)?.is_some()
            || divergence
                .as_ref()
                .is_some_and(|d| d.what == DivergenceKind::Events)
        {
            ours = false;
        }
    } else if ours && divergence.is_none() {
        divergence = verify_checkpoints(store, manifest, rom)?;
    }
    if !ours {
        // Its checkpoints are the other runtime's; don't restore them.
        let trace = match manifest.trace {
            Some(_) => verify_trace(store, manifest)?,
            None => None,
        };
        return Ok(Checked::Stale {
            frames: report.frames_checked,
            divergence: trace.or(divergence),
            runtime: describe_runtime(&manifest.runtime),
        });
    }
    let divergence = match divergence {
        None if manifest.trace.is_some() => verify_trace(store, manifest)?,
        d => d,
    };
    Ok(Checked::Replayed {
        frames: report.frames_checked,
        divergence,
        recorded_by,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrom;
    use gp_evidence::input_log::generate;

    fn record_as(
        store: &Store,
        rom: &[u8],
        input: &InputLog,
        name: &str,
        created_at: &str,
    ) -> Recorded {
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
                input,
                checkpoint_interval: 30,
                provenance: Provenance::Scripted,
                name: name.into(),
                created_at: created_at.into(),
                notes: Some(format!("{name} at {created_at}")),
            },
        )
        .unwrap()
    }

    fn record_named(store: &Store, frames: usize, name: &str, created_at: &str) -> Recorded {
        let input = generate(frames, &["a@10-40", "right+b@50-"]).unwrap();
        record_as(store, &testrom::build(), &input, name, created_at)
    }

    fn record_test_rom(store: &Store, frames: usize) -> Recorded {
        record_named(store, frames, "test", "2026-10-27T00:00:00Z")
    }

    fn record_rom(store: &Store, rom: &[u8], input: &InputLog) -> Recorded {
        record_as(store, rom, input, "test", "2026-10-27T00:00:00Z")
    }

    #[test]
    fn records_and_replays_without_divergence() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rec = record_test_rom(&store, 120);
        assert_eq!(rec.manifest.frames, 120);
        assert_eq!(rec.manifest.checkpoints.len(), 5); // frames 0, 30, 60, 90, 120
        assert!(rec.lag_frames > 40, "the test ROM lags every other frame");
        let rom = testrom::build();
        let full = replay(&store, &rec.manifest, &rom, None).unwrap();
        assert_eq!(full.divergence, None);
        assert_eq!(full.frames_checked, 120);
        let tail = replay(&store, &rec.manifest, &rom, Some(60)).unwrap();
        assert_eq!(tail.divergence, None);
        assert_eq!(tail.frames_checked, 60);
        let listed = recordings::list(&store).unwrap();
        assert_eq!(listed[0].id, rec.id);
    }

    #[test]
    fn a_reset_in_the_input_log_records_and_replays() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rom = testrom::build();
        let plain = generate(120, &["a@10-40", "right+b@50-"]).unwrap();
        let mut reset = plain.clone();
        reset.frames[45].reset = true;
        let with = record_rom(&store, &rom, &reset);
        let without = record_rom(&store, &rom, &plain);
        assert_ne!(with.id, without.id);
        // The stored log keeps the reset, in format 1, after recording
        // replaces the polled flags.
        let (stored, hashes) = load(&store, &with.manifest).unwrap();
        assert!(stored.frames[45].reset);
        assert_eq!(stored.frames.iter().filter(|f| f.reset).count(), 1);
        let (_, plain_hashes) = load(&store, &without.manifest).unwrap();
        assert_eq!(hashes[..46], plain_hashes[..46], "the same up to the reset");
        assert_ne!(hashes[46], plain_hashes[46], "the reset changes frame 46");
        // Replay from power-on and from the checkpoints after the reset.
        let full = replay(&store, &with.manifest, &rom, None).unwrap();
        assert_eq!(full.divergence, None);
        let tail = replay(&store, &with.manifest, &rom, Some(60)).unwrap();
        assert_eq!(tail.divergence, None);
    }

    #[test]
    fn recording_twice_gives_the_same_id() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let ra = record_named(
            &Store::open(a.path()).unwrap(),
            90,
            "one",
            "2026-10-27T00:00:00Z",
        );
        let rb = record_named(
            &Store::open(b.path()).unwrap(),
            90,
            "two",
            "2026-10-28T12:34:56Z",
        );
        assert_eq!(
            ra.id, rb.id,
            "same ROM, input, runtime and power-on must give the same ID whatever the name, time and notes"
        );
        assert!(!ra.existing && !rb.existing);
    }

    #[test]
    fn a_new_name_for_the_same_run_adds_an_index_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let first = record_named(&store, 60, "first", "2026-10-27T00:00:00Z");
        let again = record_named(&store, 60, "again", "2026-10-28T00:00:00Z");
        assert_eq!(first.id, again.id);
        assert!(again.existing);
        let listed = recordings::list(&store).unwrap();
        assert_eq!(
            listed.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["first", "again"]
        );
        assert_eq!(listed[1].created_at, "2026-10-28T00:00:00Z");
        assert_eq!(
            listed[1].notes.as_deref(),
            Some("again at 2026-10-28T00:00:00Z")
        );
        // A different power-on state is a different run.
        let rom = testrom::build();
        let input = generate(60, &["a@10-40", "right+b@50-"]).unwrap();
        let ones = record(
            &store,
            RecordRequest {
                rom: &rom,
                source: SourceRef::NesRom {
                    sha256: gp_evidence::source::identify_nes(&rom).unwrap().sha256,
                    mapper: 0,
                },
                source_id: None,
                power_on: PowerOn {
                    ram_init: gp_evidence::manifest::RamInit::Ones,
                    ..PowerOn::default()
                },
                input: &input,
                checkpoint_interval: 30,
                provenance: Provenance::Scripted,
                name: "first".into(),
                created_at: "2026-10-27T00:00:00Z".into(),
                notes: None,
            },
        )
        .unwrap();
        assert_ne!(ones.id, first.id);
        assert_eq!(
            ones.manifest.power_on.ram_init,
            gp_evidence::manifest::RamInit::Ones
        );
        assert_eq!(
            replay(&store, &ones.manifest, &rom, None)
                .unwrap()
                .divergence,
            None
        );
    }

    #[test]
    fn detects_a_changed_input_log() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rec = record_test_rom(&store, 90);
        // Tamper: a different input log under the same manifest.
        let other = generate(90, &["a@10-20"]).unwrap();
        let mut manifest = rec.manifest.clone();
        let mut observed = other.clone();
        for (o, f) in observed.frames.iter_mut().zip(
            InputLog::parse(
                std::str::from_utf8(&store.get(&rec.manifest.input_log).unwrap()).unwrap(),
            )
            .unwrap()
            .frames,
        ) {
            o.polled = f.polled;
        }
        manifest.input_log = store.put_bytes(observed.to_text().as_bytes()).unwrap();
        let report = replay(&store, &manifest, &testrom::build(), None).unwrap();
        let d = report.divergence.expect("a different input must diverge");
        // Input frame 21 is the first that differs, but frame 22, which it
        // runs, is a lag frame. Held buttons aren't console state, so nothing
        // differs until input frame 22 runs frame 23, which reads the
        // controller into RAM.
        assert_eq!(d.frame, 23);
        assert_eq!(d.what, DivergenceKind::Ram);
    }

    fn input_of(store: &Store, manifest: &RecordingManifest) -> InputLog {
        InputLog::parse(std::str::from_utf8(&store.get(&manifest.input_log).unwrap()).unwrap())
            .unwrap()
    }

    #[test]
    fn reports_a_ram_divergence() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rec = record_test_rom(&store, 90);
        let mut hashes =
            frame_hashes::decode(&store.get(&rec.manifest.frame_hashes.unwrap()).unwrap()).unwrap();
        hashes[50].ram = Hash::of(b"not this RAM");
        let mut manifest = rec.manifest.clone();
        manifest.frame_hashes = Some(store.put_bytes(&frame_hashes::encode(&hashes)).unwrap());
        let d = replay(&store, &manifest, &testrom::build(), None)
            .unwrap()
            .divergence
            .unwrap();
        assert_eq!((d.frame, d.what), (50, DivergenceKind::Ram));
    }

    #[test]
    fn refuses_frame_hashes_of_another_format() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rec = record_test_rom(&store, 60);
        let mut bytes = store.get(&rec.manifest.frame_hashes.unwrap()).unwrap();
        bytes[4..8].copy_from_slice(&0u32.to_le_bytes());
        let mut manifest = rec.manifest.clone();
        manifest.frame_hashes = Some(store.put_bytes(&bytes).unwrap());
        let err = replay(&store, &manifest, &testrom::build(), None).unwrap_err();
        assert!(matches!(err, RecordingError::OldFrameHashes(0)), "{err}");
    }

    #[test]
    fn reports_a_polling_divergence() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rec = record_test_rom(&store, 90);
        let mut input = input_of(&store, &rec.manifest);
        input.frames[40].polled = !input.frames[40].polled;
        let mut manifest = rec.manifest.clone();
        manifest.input_log = store.put_bytes(input.to_text().as_bytes()).unwrap();
        let d = replay(&store, &manifest, &testrom::build(), None)
            .unwrap()
            .divergence
            .unwrap();
        // Input frame 40 produces frame 41.
        assert_eq!((d.frame, d.what), (41, DivergenceKind::Polled));
    }

    #[test]
    fn every_checkpoint_restores_to_its_recorded_state() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rec = record_test_rom(&store, 120);
        let rom = testrom::build();
        assert_eq!(
            verify_checkpoints(&store, &rec.manifest, &rom).unwrap(),
            None
        );
        // A checkpoint taken at another frame is caught.
        let mut manifest = rec.manifest.clone();
        manifest.checkpoints[2].snapshot = manifest.checkpoints[1].snapshot;
        let d = verify_checkpoints(&store, &manifest, &rom)
            .unwrap()
            .unwrap();
        assert_eq!(d.frame, 60);
    }

    #[test]
    fn refuses_checkpoints_from_another_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rec = record_test_rom(&store, 90);
        assert!(made_by_this_runtime(&rec.manifest));
        let mut manifest = rec.manifest.clone();
        manifest.runtime.version = "0 (tetanes-core 0.16.0 + patch 000000000000)".into();
        assert!(!made_by_this_runtime(&manifest));
        let rom = testrom::build();
        // From power-on it still replays, and here it still matches.
        let full = replay(&store, &manifest, &rom, None).unwrap();
        assert_eq!(full.divergence, None);
        assert_eq!(full.frames_checked, 90);
        // Its snapshots belong to the other runtime.
        let err = replay(&store, &manifest, &rom, Some(60)).unwrap_err();
        assert!(matches!(err, RecordingError::OtherRuntime { .. }), "{err}");
        assert!(err.to_string().contains("tetanes-core 0.16.0"), "{err}");
        assert!(matches!(
            verify_checkpoints(&store, &manifest, &rom),
            Err(RecordingError::OtherRuntime { .. })
        ));
    }

    /// The manifest as a build differing only in its fork patch would see it.
    fn with_other_patch(manifest: &RecordingManifest) -> RecordingManifest {
        let mut m = manifest.clone();
        let (head, _) = m.runtime.version.split_once(" + patch ").unwrap();
        m.runtime.version = format!("{head} + patch 000000000000)");
        m
    }

    #[test]
    fn tells_a_patch_only_runtime_from_another() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rec = record_test_rom(&store, 60);
        assert_eq!(lineage(&rec.manifest), Lineage::Same);
        assert_eq!(
            lineage(&with_other_patch(&rec.manifest)),
            Lineage::PatchOnly
        );
        let other = |version: &str| {
            let mut m = rec.manifest.clone();
            m.runtime.version = version.into();
            lineage(&m)
        };
        let ours = runtime_ref().version;
        let (_, rest) = ours.split_once(' ').unwrap();
        // Another behaviour version, or another upstream release.
        assert_eq!(other(&format!("999 {rest}")), Lineage::Other);
        assert_eq!(
            other("2 (tetanes-core 0.16.0 + patch 000000000000)"),
            Lineage::Other
        );
        assert_eq!(other("an older gp-nes"), Lineage::Other);
        let mut m = with_other_patch(&rec.manifest);
        m.runtime.name = "mesen".into();
        assert_eq!(lineage(&m), Lineage::Other);
    }

    #[test]
    fn a_patch_only_runtime_keeps_checkpoints_that_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rec = record_test_rom(&store, 90);
        let rom = testrom::build();
        let manifest = with_other_patch(&rec.manifest);
        assert!(keeps_checkpoints(&store, &manifest, &rom).unwrap());
        let tail = replay(&store, &manifest, &rom, Some(60)).unwrap();
        assert_eq!((tail.divergence, tail.frames_checked), (None, 30));
        let checked = check(&store, &manifest, &rom).unwrap();
        assert_eq!(checked.result(), "ok");
        assert!(matches!(
            checked,
            Checked::Replayed {
                recorded_by: Some(_),
                ..
            }
        ));
        assert!(
            checked
                .line("abc", "test")
                .contains("differs only in its fork patch")
        );
    }

    #[test]
    fn a_patch_only_runtime_drops_checkpoints_that_dont_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rec = record_test_rom(&store, 90);
        let rom = testrom::build();
        // A snapshot this runtime saves differently: one byte more, which
        // tetanes-core ignores on load (trailing bytes), so it restores
        // but doesn't save back to the same bytes.
        let mut manifest = with_other_patch(&rec.manifest);
        let mut longer = store.get(&manifest.checkpoints[2].snapshot).unwrap();
        longer.push(0);
        manifest.checkpoints[2].snapshot = store.put_bytes(&longer).unwrap();
        assert_eq!(
            verify_checkpoints(&store, &manifest, &rom).unwrap(),
            Some(Divergence {
                frame: 60,
                what: DivergenceKind::Snapshot
            })
        );
        assert!(!keeps_checkpoints(&store, &manifest, &rom).unwrap());
        let err = replay(&store, &manifest, &rom, Some(60)).unwrap_err();
        assert!(
            matches!(
                err,
                RecordingError::CheckpointDoesNotRoundTrip { frame: 60, .. }
            ),
            "{err}"
        );
        // The other checkpoints still restore.
        assert_eq!(
            replay(&store, &manifest, &rom, Some(30))
                .unwrap()
                .divergence,
            None
        );
        // It replays from power-on, so it's stale rather than diverged.
        let checked = check(&store, &manifest, &rom).unwrap();
        assert_eq!(checked.result(), "stale", "{checked:?}");
        // Under the runtime that made it, the same snapshot is damage.
        let mut ours = manifest.clone();
        ours.runtime = runtime_ref();
        assert_eq!(check(&store, &ours, &rom).unwrap().result(), "diverged");
        // Stale too when it also diverges from power-on, as `gp replay` says.
        let mut input = input_of(&store, &manifest);
        input.frames[40].polled = !input.frames[40].polled;
        manifest.input_log = store.put_bytes(input.to_text().as_bytes()).unwrap();
        assert!(!keeps_checkpoints(&store, &manifest, &rom).unwrap());
        let checked = check(&store, &manifest, &rom).unwrap();
        assert_eq!(checked.result(), "stale", "{checked:?}");
        // With checkpoints that round-trip, the same divergence fails.
        let mut good = with_other_patch(&rec.manifest);
        good.input_log = manifest.input_log;
        assert_eq!(check(&store, &good, &rom).unwrap().result(), "diverged");
    }

    #[test]
    fn refuses_bad_requests() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rec = record_test_rom(&store, 90);
        let err = replay(&store, &rec.manifest, &testrom::build(), Some(45)).unwrap_err();
        assert!(err.to_string().contains("0, 30, 60, 90"), "{err}");
        let mut input = generate(10, &[]).unwrap();
        input.controllers = 3;
        let rom = testrom::build();
        fn req<'a>(rom: &'a [u8], input: &'a InputLog, interval: u32) -> RecordRequest<'a> {
            RecordRequest {
                rom,
                source: SourceRef::NesRom {
                    sha256: gp_evidence::source::identify_nes(rom).unwrap().sha256,
                    mapper: 0,
                },
                source_id: None,
                power_on: PowerOn::default(),
                input,
                checkpoint_interval: interval,
                provenance: Provenance::Scripted,
                name: "bad".into(),
                created_at: "2026-10-27T00:00:00Z".into(),
                notes: None,
            }
        }
        assert!(matches!(
            record(&store, req(&rom, &input, 30)),
            Err(RecordingError::TooManyControllers(3))
        ));
        input.controllers = 1;
        assert!(matches!(
            record(&store, req(&rom, &input, 0)),
            Err(RecordingError::ZeroInterval)
        ));
    }

    #[test]
    fn counts_only_new_bytes_as_stored() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let first = record_test_rom(&store, 90);
        assert_eq!(first.new_bytes, first.bytes);
        let second = record_test_rom(&store, 90);
        // Same content, same ID: nothing new to store.
        assert_eq!(second.id, first.id);
        assert_eq!(second.new_bytes, 0);
    }

    fn record_capturing(store: &Store, holds: &[&str], capture: Capture) -> Recorded {
        let rom = testrom::build();
        let input = generate(150, holds).unwrap();
        record_with(
            store,
            RecordRequest {
                rom: &rom,
                source: SourceRef::NesRom {
                    sha256: gp_evidence::source::identify_nes(&rom).unwrap().sha256,
                    mapper: 0,
                },
                source_id: None,
                power_on: PowerOn::default(),
                input: &input,
                checkpoint_interval: 30,
                provenance: Provenance::Scripted,
                name: "capture".into(),
                created_at: "2026-10-27T00:00:00Z".into(),
                notes: None,
            },
            capture,
        )
        .unwrap()
    }

    #[test]
    fn traces_and_events_add_data_and_change_nothing_else() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let holds = ["a@10-40", "right+b@50-"];
        let bare = record_capturing(&store, &holds, Capture::default());
        let full = record_capturing(
            &store,
            &holds,
            Capture {
                trace: Some(64),
                events: Some(40),
            },
        );
        assert_eq!(bare.manifest.frame_hashes, full.manifest.frame_hashes);
        assert_eq!(bare.manifest.checkpoints, full.manifest.checkpoints);
        assert_ne!(bare.id, full.id);
        assert_eq!((bare.trace_bytes, bare.events_bytes), (0, 0));
        let trace = full.manifest.trace.as_ref().unwrap();
        // Frames 0 to 150: three chunks of 64, four of 40.
        assert_eq!(trace.chunks.len(), 3);
        assert_eq!(full.manifest.events.as_ref().unwrap().chunks.len(), 4);
        assert!(full.trace_bytes > 0 && full.events_bytes > 0);
        assert_eq!(
            field_table(&store, &full.manifest).unwrap(),
            canonical::field_table()
        );

        let rom = testrom::build();
        assert_eq!(verify_trace(&store, &full.manifest).unwrap(), None);
        assert!(matches!(
            verify_trace(&store, &bare.manifest),
            Err(RecordingError::NoTrace)
        ));
        for frame in [0, 1, 45, 63, 64, 120, 150] {
            let traced = trace_state(&store, &full.manifest, frame).unwrap().unwrap();
            assert_eq!(
                traced,
                replayed_state(&store, &full.manifest, &rom, frame).unwrap(),
                "frame {frame}"
            );
        }
        assert_eq!(trace_state(&store, &bare.manifest, 5).unwrap(), None);
        // A scan gives the same states in order, across chunks, and stops
        // when asked.
        let mut seen = Vec::new();
        assert!(
            scan_trace(&store, &full.manifest, 60..=130, |frame, state| {
                assert_eq!(
                    state,
                    trace_state(&store, &full.manifest, frame).unwrap().unwrap()
                );
                seen.push(frame);
                frame < 129
            })
            .unwrap()
        );
        assert_eq!(seen, (60..=129).collect::<Vec<_>>());
        assert!(!scan_trace(&store, &bare.manifest, 0..=5, |_, _| true).unwrap());
        assert!(scan_trace(&store, &full.manifest, 0..=151, |_, _| true).is_err());
        assert!(matches!(
            trace_state(&store, &full.manifest, 151),
            Err(RecordingError::PastEnd { .. })
        ));
        for range in [0..151, 30..71, 61..62, 140..400] {
            let stored = stored_events(&store, &full.manifest, range.clone())
                .unwrap()
                .unwrap();
            assert_eq!(
                stored,
                replayed_events(&store, &full.manifest, &rom, range.clone()).unwrap(),
                "{range:?}"
            );
            assert!(stored.iter().all(|e| range.contains(&e.frame)));
        }
        assert_eq!(stored_events(&store, &bare.manifest, 0..10).unwrap(), None);

        // Replay checks the events, from power-on and from a checkpoint.
        for from in [None, Some(60)] {
            let report = replay(&store, &full.manifest, &rom, from).unwrap();
            assert_eq!(report.divergence, None);
        }
        assert!(matches!(
            check(&store, &full.manifest, &rom).unwrap(),
            Checked::Replayed {
                divergence: None,
                ..
            }
        ));
    }

    #[test]
    fn detects_a_trace_or_event_stream_that_doesnt_match() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let a = record_capturing(&store, &["a@10-40"], Capture::ALL);
        let b = record_capturing(&store, &["a@10-80"], Capture::ALL);
        let rom = testrom::build();
        // b's states and events differ from a's from the frame after its
        // input does, so b's second chunks don't match a's hashes.
        let mut m = a.manifest.clone();
        m.trace.as_mut().unwrap().chunks[1] = b.manifest.trace.as_ref().unwrap().chunks[1];
        let d = verify_trace(&store, &m).unwrap().unwrap();
        assert_eq!(d.what, DivergenceKind::Trace);
        assert!((64..128).contains(&d.frame), "{d:?}");
        assert_eq!(
            check(&store, &m, &rom).unwrap(),
            Checked::Replayed {
                frames: 150,
                divergence: Some(d.clone()),
                recorded_by: None
            }
        );
        // A state served from the damaged chunk is refused, not passed on.
        assert!(matches!(
            trace_state(&store, &m, d.frame),
            Err(RecordingError::TraceMismatch(f)) if f == d.frame
        ));
        assert!(trace_state(&store, &m, 10).unwrap().is_some());
        // A stale recording's trace is checked too, and damage there fails
        // the check rather than counting as stale.
        let mut stale = m.clone();
        stale.runtime.version = "0 (an older gp-nes)".into();
        let checked = check(&store, &stale, &rom).unwrap();
        assert_eq!(checked.result(), "diverged", "{checked:?}");
        assert!(matches!(
            checked,
            Checked::Stale {
                divergence: Some(Divergence {
                    what: DivergenceKind::Trace,
                    ..
                }),
                ..
            }
        ));
        assert!(checked.line("id", "a").starts_with("DIVERGED"));
        let mut stale = a.manifest.clone();
        stale.runtime.version = "0 (an older gp-nes)".into();
        assert_eq!(check(&store, &stale, &rom).unwrap().result(), "stale");
        // Another emulator's recording (Mesen's, from gp oracle record) is
        // never replayed, only its trace checked, and damage still fails.
        let mut mesen = a.manifest.clone();
        mesen.runtime.name = "mesen2".into();
        let checked = check(&store, &mesen, &rom).unwrap();
        assert!(
            matches!(
                &checked,
                Checked::OtherRuntime {
                    divergence: None,
                    ..
                }
            ),
            "{checked:?}"
        );
        assert_eq!(checked.result(), "other");
        assert!(checked.line("id", "a").starts_with("other "));
        let mut mesen = m.clone();
        mesen.runtime.name = "mesen2".into();
        let checked = check(&store, &mesen, &rom).unwrap();
        assert_eq!(checked.result(), "diverged", "{checked:?}");
        // A chunk whose header claims more frames than its place in the
        // manifest is refused before it's decoded.
        let mut forged = store
            .get(&a.manifest.trace.as_ref().unwrap().chunks[1])
            .unwrap();
        forged[16..20].copy_from_slice(&80_000u32.to_le_bytes());
        let mut m = a.manifest.clone();
        m.trace.as_mut().unwrap().chunks[1] = store.put_bytes(&forged).unwrap();
        assert!(matches!(
            trace_state(&store, &m, 70),
            Err(RecordingError::Chunk(ChunkError::Invalid(_)))
        ));
        // A chunk in the wrong place is corrupt rather than a divergence.
        let mut m = a.manifest.clone();
        m.trace.as_mut().unwrap().chunks.swap(0, 1);
        assert!(matches!(
            verify_trace(&store, &m),
            Err(RecordingError::Chunk(ChunkError::Invalid(_)))
        ));

        let mut m = a.manifest.clone();
        m.events.as_mut().unwrap().chunks[1] = b.manifest.events.as_ref().unwrap().chunks[1];
        let d = replay(&store, &m, &rom, None).unwrap().divergence.unwrap();
        assert_eq!(d.what, DivergenceKind::Events);
        assert!((64..128).contains(&d.frame), "{d:?}");
        assert_eq!(check(&store, &m, &rom).unwrap().result(), "diverged");
        // Under a patch-only runtime, events alone may differ (a new hook,
        // say): stale, not diverged.
        let patched = with_other_patch(&m);
        let checked = check(&store, &patched, &rom).unwrap();
        assert_eq!(checked.result(), "stale", "{checked:?}");
    }

    #[test]
    fn records_and_replays_two_controllers() {
        use gp_evidence::input_log::{InputFrame, button};
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let rom = testrom::build_port2_alone();
        let mut input = InputLog::new(2);
        for f in 0..120u8 {
            let p1 = if f % 5 < 2 { button::A } else { button::RIGHT };
            let p2 = if f % 3 == 0 {
                button::B | button::UP
            } else {
                0
            };
            input.frames.push(InputFrame {
                polled: false,
                pads: [p1, p2, 0, 0],
                reset: false,
            });
        }
        let rec = record_rom(&store, &rom, &input);
        assert_eq!(input_of(&store, &rec.manifest).controllers, 2);
        // build_port2_alone lags on one NMI count in four.
        assert!((25..=35).contains(&rec.lag_frames), "{}", rec.lag_frames);
        assert_eq!(
            replay(&store, &rec.manifest, &rom, None)
                .unwrap()
                .divergence,
            None
        );
        assert_eq!(
            verify_checkpoints(&store, &rec.manifest, &rom).unwrap(),
            None
        );
        // A change to controller 2 alone is caught. Port 1 is untouched.
        let mut changed = input_of(&store, &rec.manifest);
        changed.frames[70].pads[1] ^= button::START;
        let mut manifest = rec.manifest.clone();
        manifest.input_log = store.put_bytes(changed.to_text().as_bytes()).unwrap();
        let d = replay(&store, &manifest, &rom, None)
            .unwrap()
            .divergence
            .expect("a different controller 2 input must diverge");
        assert_eq!(d.frame, 71);
    }
}
