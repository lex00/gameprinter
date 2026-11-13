//! A recording's states as one list, and recordings built from a list of
//! states rather than by running `gp-nes`: how a Mesen capture is stored,
//! and how the mutation tests make a recording that differs on purpose.

use gp_evidence::chunk::{self, ChunkError};
use gp_evidence::events::{self, Event, EventWriter};
use gp_evidence::frame_hashes::{self, FrameHash};
use gp_evidence::manifest::{
    EventsRef, PowerOn, Provenance, RECORDING_SCHEMA, RecordingManifest, RuntimeRef, SourceRef,
    TraceRef,
};
use gp_evidence::recordings::{self, IndexEntry};
use gp_evidence::trace::{self, FieldTable, TraceWriter};
use gp_evidence::{Hash, InputLog, Store};

#[derive(Debug, thiserror::Error)]
pub enum StatesError {
    #[error(transparent)]
    Store(#[from] gp_evidence::store::StoreError),
    #[error(transparent)]
    Chunk(#[from] ChunkError),
    #[error(transparent)]
    Recordings(#[from] recordings::RecordingsError),
    #[error("{0}")]
    Invalid(String),
}

/// Every frame's state, 0 to the last, and the table that splits them.
#[derive(Debug, Clone)]
pub struct States {
    pub table: FieldTable,
    pub states: Vec<Vec<u8>>,
}

/// The recording's input log, as stored (with the polled flags it saw).
pub fn input_log(store: &Store, manifest: &RecordingManifest) -> Result<InputLog, StatesError> {
    let text = String::from_utf8(store.get(&manifest.input_log)?)
        .map_err(|_| StatesError::Invalid("the input log isn't text".into()))?;
    InputLog::parse(&text).map_err(|e| StatesError::Invalid(e.to_string()))
}

/// Every state in a recording's trace, checked against its frame hashes.
pub fn load(store: &Store, manifest: &RecordingManifest) -> Result<States, StatesError> {
    let t = manifest.trace.as_ref().ok_or_else(|| {
        StatesError::Invalid(
            "the recording has no frame trace; record it again without --no-trace".into(),
        )
    })?;
    let table = FieldTable::decode(&store.get(&t.fields)?)?;
    // Grown chunk by chunk: each decode is bounded by the field table, so a
    // manifest's frame count never sizes an allocation on its own.
    let mut states = Vec::new();
    for (i, id) in t.chunks.iter().enumerate() {
        let expected = chunk::chunk_range(i, t.chunk_frames, manifest.frames);
        trace::decode_each(&store.get(id)?, &expected, &table, |_, state| {
            states.push(state.to_vec())
        })?;
    }
    if states.len() as u64 != manifest.frames + 1 {
        return Err(StatesError::Invalid(format!(
            "the trace holds {} states for {} frames",
            states.len(),
            manifest.frames
        )));
    }
    if let Some(h) = &manifest.frame_hashes {
        let hashes = frame_hashes::decode(&store.get(h)?)
            .map_err(|e| StatesError::Invalid(e.to_string()))?;
        for (frame, (state, hash)) in states.iter().zip(&hashes).enumerate() {
            if Hash::of(state) != hash.state {
                return Err(StatesError::Invalid(format!(
                    "the trace's state at frame {frame} doesn't match its frame hash"
                )));
            }
        }
    }
    Ok(States { table, states })
}

/// What [`store`] needs besides the states.
pub struct Made<'a> {
    pub source: SourceRef,
    pub source_id: Option<String>,
    pub runtime: RuntimeRef,
    pub power_on: PowerOn,
    /// The input log with the polled flags the runtime saw.
    pub input: &'a InputLog,
    pub provenance: Provenance,
    pub name: String,
    pub created_at: String,
    pub notes: Option<String>,
}

/// Stores a recording from its states (and events, one list per frame),
/// with a trace and frame hashes and no checkpoints: only the runtime that
/// made it could restore one. Returns the recording's ID and manifest.
pub fn store(
    store: &Store,
    made: Made<'_>,
    states: &States,
    events_by_frame: Option<&[Vec<Event>]>,
) -> Result<(Hash, RecordingManifest), StatesError> {
    let frames = made.input.frames.len() as u64;
    if states.states.len() as u64 != frames + 1 {
        return Err(StatesError::Invalid(format!(
            "{} states for an input log of {frames} frames",
            states.states.len()
        )));
    }
    let chunk_frames = chunk::DEFAULT_CHUNK_FRAMES;
    let fields = store.put_bytes(&states.table.encode())?;
    let mut writer = TraceWriter::new(chunk_frames, fields);
    let mut chunks = Vec::new();
    let mut hashes = Vec::with_capacity(states.states.len());
    for state in &states.states {
        let split = states.table.split(state)?;
        let ram_bytes = split
            .iter()
            .find(|(n, _)| *n == "ram")
            .map_or(&[][..], |(_, v)| *v);
        hashes.push(FrameHash {
            state: Hash::of(state),
            ram: Hash::of(ram_bytes),
        });
        if let Some(c) = writer.push(state)? {
            chunks.push(store.put_bytes(&c)?);
        }
    }
    if let Some(c) = writer.finish() {
        chunks.push(store.put_bytes(&c)?);
    }
    let events = match events_by_frame {
        Some(by_frame) => {
            let mut w = EventWriter::new(chunk_frames);
            let mut ev_chunks = Vec::new();
            for frame in by_frame {
                if let Some(c) = w.push(frame) {
                    ev_chunks.push(store.put_bytes(&c)?);
                }
            }
            if let Some(c) = w.finish() {
                ev_chunks.push(store.put_bytes(&c)?);
            }
            Some(EventsRef {
                schema: events::SCHEMA,
                chunk_frames,
                chunks: ev_chunks,
            })
        }
        None => None,
    };
    let manifest = RecordingManifest {
        schema: RECORDING_SCHEMA,
        source: made.source,
        runtime: made.runtime,
        power_on: made.power_on,
        frames,
        input_log: store.put_bytes(made.input.to_text().as_bytes())?,
        // No checkpoints; the interval says so by covering the whole run.
        checkpoint_interval: frames.max(1) as u32,
        checkpoints: Vec::new(),
        frame_hashes: Some(store.put_bytes(&frame_hashes::encode(&hashes))?),
        trace: Some(TraceRef {
            schema: trace::SCHEMA,
            chunk_frames,
            fields,
            chunks,
        }),
        events,
        provenance: made.provenance,
    };
    manifest
        .validate()
        .map_err(|e| StatesError::Invalid(e.to_string()))?;
    let id = store.put_bytes(manifest.to_json().as_bytes())?;
    recordings::append(
        store,
        &IndexEntry {
            id,
            created_at: made.created_at,
            source: made.source_id,
            frames,
            name: made.name,
            notes: made.notes,
            golden: false,
        },
    )?;
    Ok((id, manifest))
}
