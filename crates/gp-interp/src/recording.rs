//! Recording and replaying model runs in the evidence store, as `gp-nes`
//! does for ROMs (docs/recording.md, docs/interpreter.md).
//!
//! A model recording's source is `{"kind": "model", "name", "bundle"}`: the
//! bundle is a store object holding the model's canonical text and the
//! lifted asset files it reads ([`Bundle`]), so a replay needs nothing but
//! the store. Every frame's hashes are the BLAKE3 hash of the model's
//! canonical state (both the `state` and the `ram` hash, since a model has
//! no state outside its game logic), and each checkpoint is that canonical
//! state, which restores exactly. A model reads its input every frame, so
//! the stored input log marks every frame polled.
//!
//! A recording made now also keeps a frame trace and an event stream
//! ([`crate::modeltrace`], `gp_evidence::model_events`): the trace is a view
//! of the state by model path, and the frame hash's `state` is that view's
//! hash. The `ram` hash stays the canonical state's. A recording without a
//! trace (made before this) checks by the canonical hash alone.

use std::time::Instant;

use gp_evidence::frame_hashes::{self, FrameHash};
use gp_evidence::manifest::{
    Checkpoint, EventsRef, PowerOn, Provenance, RECORDING_SCHEMA, RecordingManifest, RuntimeRef,
    SourceRef, TraceRef,
};
use gp_evidence::model_events::{ModelEvent, ModelEventWriter};
use gp_evidence::recordings::{self, IndexEntry};
use gp_evidence::trace::{self, FieldTable, TraceWriter};
use gp_evidence::{Hash, InputLog, Store};
use gp_runtime::InputFrame;
use gp_sim::Exts;
use std::sync::Arc;

use crate::lifted::Bundle;
use crate::modeltrace::{Encoder, Layout};
use crate::runtime::Interp;

#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error(transparent)]
    Store(#[from] gp_evidence::store::StoreError),
    #[error(transparent)]
    Recordings(#[from] recordings::RecordingsError),
    #[error("the input log is empty")]
    EmptyInput,
    #[error("the checkpoint interval must be at least 1 frame")]
    ZeroInterval,
    #[error("the model: {0}")]
    Model(String),
    #[error("not a model recording")]
    NotAModel,
    #[error("recording data is inconsistent: {0}")]
    Corrupt(String),
    #[error(transparent)]
    Chunk(#[from] gp_evidence::chunk::ChunkError),
}

/// The trace's field table for a run of `input` on the bundle's model: every
/// path the run names.
fn layout_of(
    bundle: &Bundle,
    input: &InputLog,
    exts: &Arc<Exts>,
) -> Result<FieldTable, RecordingError> {
    let mut rt = Interp::from_bundle(bundle)
        .map_err(RecordingError::Model)?
        .with_exts(Arc::clone(exts));
    let mut layout = Layout::default();
    layout
        .see(rt.spec(), rt.world())
        .map_err(RecordingError::Model)?;
    for f in &input.frames {
        rt.advance(&input_frame(f)).map_err(RecordingError::Model)?;
        layout
            .see(rt.spec(), rt.world())
            .map_err(RecordingError::Model)?;
    }
    Ok(layout.table())
}

/// A frame's events as the stream stores them. The frame's own `frame`
/// event, which ends every frame in the protocol, isn't stored.
fn model_events(rt: &Interp, raised: &[gp_sim::Event]) -> Vec<ModelEvent> {
    raised
        .iter()
        .map(|e| ModelEvent {
            frame: e.frame,
            kind: rt.spec().events[e.kind.0].clone(),
            entity: e.entity.map(|i| i as u64),
        })
        .collect()
}

/// The runtime as recordings name it: `gp-interp`, its behaviour version
/// and its canonical state's version.
pub fn runtime_ref() -> RuntimeRef {
    runtime_ref_with_hooks("")
}

/// The runtime with `ext` hooks: their label (the hook crate, its version
/// and its code's hash, from `gp_exts`) goes in the version after a `;`,
/// so recordings made with other hooks, or none, replay as stale.
pub fn runtime_ref_with_hooks(hooks: &str) -> RuntimeRef {
    RuntimeRef {
        name: crate::RUNTIME_NAME.into(),
        version: if hooks.is_empty() {
            format!(
                "{} (state {} {})",
                crate::BEHAVIOUR_VERSION,
                String::from_utf8_lossy(gp_sim::world::MAGIC),
                gp_sim::world::VERSION
            )
        } else {
            format!(
                "{} (state {} {}; hooks {hooks})",
                crate::BEHAVIOUR_VERSION,
                String::from_utf8_lossy(gp_sim::world::MAGIC),
                gp_sim::world::VERSION
            )
        },
        commit: None,
    }
}

/// What to record. As for `gp-nes`, the run goes in the manifest and
/// decides the ID; `name`, `created_at` and `notes` go only in the index.
pub struct RecordRequest<'a> {
    pub bundle: &'a Bundle,
    pub input: &'a InputLog,
    pub checkpoint_interval: u32,
    pub provenance: Provenance,
    pub name: String,
    pub created_at: String,
    pub notes: Option<String>,
}

#[derive(Debug)]
pub struct Recorded {
    pub id: Hash,
    pub manifest: RecordingManifest,
    pub frames_per_second: f64,
    pub bytes: u64,
    pub new_bytes: u64,
    pub existing: bool,
}

pub fn input_frame(f: &gp_evidence::InputFrame) -> InputFrame {
    InputFrame {
        buttons: f.buttons_word(),
        axes: Vec::new(),
        reset: f.reset,
    }
}

fn frame_hash(rt: &Interp) -> FrameHash {
    let h = Hash::from_bytes(rt.world().hash());
    FrameHash { state: h, ram: h }
}

pub fn record(store: &Store, req: RecordRequest<'_>) -> Result<Recorded, RecordingError> {
    record_with_hooks(store, req, Arc::new(Exts::new()), "")
}

/// [`record`], running these `ext` hooks, whose label goes in the
/// runtime's version.
pub fn record_with_hooks(
    store: &Store,
    req: RecordRequest<'_>,
    exts: Arc<Exts>,
    hooks: &str,
) -> Result<Recorded, RecordingError> {
    if req.input.frames.is_empty() {
        return Err(RecordingError::EmptyInput);
    }
    if req.checkpoint_interval == 0 {
        return Err(RecordingError::ZeroInterval);
    }
    let mut rt = Interp::from_bundle(req.bundle)
        .map_err(RecordingError::Model)?
        .with_exts(Arc::clone(&exts));
    // The trace's field table names every path the run ever names, so the
    // run goes once before it is recorded.
    let table = layout_of(req.bundle, req.input, &exts)?;
    let encoder = Encoder::new(&table).map_err(RecordingError::Model)?;
    let chunk_frames = gp_evidence::chunk::DEFAULT_CHUNK_FRAMES;
    let (mut bytes, mut new_bytes) = (0u64, 0u64);
    let mut put = |data: &[u8]| -> Result<Hash, RecordingError> {
        bytes += data.len() as u64;
        if !store.contains(&Hash::of(data)) {
            new_bytes += data.len() as u64;
        }
        Ok(store.put_bytes(data)?)
    };
    let bundle = put(&req.bundle.to_bytes())?;
    let fields = put(&table.encode())?;
    let mut trace_writer = TraceWriter::new(chunk_frames, fields);
    let mut event_writer = ModelEventWriter::new(chunk_frames);
    let (mut trace_chunks, mut event_chunks) = (Vec::new(), Vec::new());
    let mut observed = req.input.clone();
    let interval = u64::from(req.checkpoint_interval);
    let mut hashes = Vec::with_capacity(observed.frames.len() + 1);
    let mut checkpoints = Vec::new();
    let start = Instant::now();
    for i in 0..=observed.frames.len() {
        let mut events = Vec::new();
        if i > 0 {
            let f = &mut observed.frames[i - 1];
            let raised = rt.advance(&input_frame(f)).map_err(RecordingError::Model)?;
            events = model_events(&rt, &raised);
            f.polled = true;
        }
        let view = encoder
            .encode(rt.spec(), rt.world())
            .map_err(RecordingError::Model)?;
        hashes.push(FrameHash {
            state: Hash::of(&view),
            ram: frame_hash(&rt).ram,
        });
        if let Some(c) = trace_writer.push(&view)? {
            trace_chunks.push(put(&c)?);
        }
        if let Some(c) = event_writer.push(&events) {
            event_chunks.push(put(&c)?);
        }
        if (i as u64).is_multiple_of(interval) {
            checkpoints.push(Checkpoint {
                frame: i as u64,
                snapshot: put(&rt.world().canonical())?,
            });
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    if let Some(c) = trace_writer.finish() {
        trace_chunks.push(put(&c)?);
    }
    if let Some(c) = event_writer.finish() {
        event_chunks.push(put(&c)?);
    }
    let manifest = RecordingManifest {
        schema: RECORDING_SCHEMA,
        source: SourceRef::Model {
            name: rt.spec().name.clone(),
            bundle,
        },
        runtime: runtime_ref_with_hooks(hooks),
        // A model has no console; the field is required, so it holds the
        // defaults, which nothing reads.
        power_on: PowerOn::default(),
        frames: observed.frames.len() as u64,
        input_log: put(observed.to_text().as_bytes())?,
        checkpoint_interval: req.checkpoint_interval,
        checkpoints,
        frame_hashes: Some(put(&frame_hashes::encode(&hashes))?),
        trace: Some(TraceRef {
            schema: trace::SCHEMA,
            chunk_frames,
            fields,
            chunks: trace_chunks,
        }),
        events: Some(EventsRef {
            schema: gp_evidence::model_events::SCHEMA,
            chunk_frames,
            chunks: event_chunks,
        }),
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
            source: Some(rt.spec().name.clone()),
            frames: manifest.frames,
            name: req.name,
            notes: req.notes,
            golden: false,
        },
    )?;
    Ok(Recorded {
        id,
        manifest,
        frames_per_second: observed.frames.len() as f64 / elapsed.max(1e-9),
        bytes,
        new_bytes,
        existing,
    })
}

/// Where a replay first disagreed with its recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divergence {
    /// The state after this frame differs.
    State(u64),
    /// The checkpoint at this frame doesn't restore to its recorded state.
    Checkpoint(u64),
    /// The event stream differs in the chunk that starts at this frame.
    Events(u64),
}

impl Divergence {
    pub fn frame(self) -> u64 {
        match self {
            Self::State(f) | Self::Checkpoint(f) | Self::Events(f) => f,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::State(_) => "the model's state differs",
            Self::Checkpoint(_) => "the checkpoint doesn't restore to the recorded state",
            Self::Events(_) => "the event stream differs from the replay's",
        }
    }
}

/// What replaying a model recording found.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub frames: u64,
    pub divergence: Option<Divergence>,
    /// The runtime that made it, when it isn't this one: the recording is
    /// stale, and a divergence means re-record it.
    pub stale: Option<String>,
    pub frames_per_second: f64,
}

impl Checked {
    /// `ok`, `diverged` or `stale`, as `gp replay --all` counts them.
    pub fn result(&self) -> &'static str {
        match (&self.stale, &self.divergence) {
            (None, None) => "ok",
            (None, Some(_)) => "diverged",
            (Some(_), _) => "stale",
        }
    }

    /// One report line, as `gp replay --all` prints it.
    pub fn line(&self, short_id: &str, name: &str) -> String {
        match (&self.stale, &self.divergence) {
            (None, None) => format!(
                "ok       {short_id}  {name}  {} frames of a model, checkpoints restore",
                self.frames
            ),
            (None, Some(d)) => format!(
                "DIVERGED {short_id}  {name}  at frame {}: {}",
                d.frame(),
                d.describe()
            ),
            (Some(r), None) => format!(
                "STALE    {short_id}  {name}  {} frames of a model match (recorded by {r})",
                self.frames
            ),
            (Some(r), Some(d)) => format!(
                "STALE    {short_id}  {name}  differs from frame {}: {} (recorded by {r})",
                d.frame(),
                d.describe()
            ),
        }
    }
}

/// The bundle a model recording names, from the store.
pub fn bundle(store: &Store, manifest: &RecordingManifest) -> Result<Bundle, RecordingError> {
    let SourceRef::Model { bundle, .. } = &manifest.source else {
        return Err(RecordingError::NotAModel);
    };
    Bundle::from_bytes(&store.get(bundle)?).map_err(RecordingError::Corrupt)
}

/// Replays a model recording from power-on, comparing every frame's hash,
/// then restores every checkpoint and compares it with its frame.
pub fn check(store: &Store, manifest: &RecordingManifest) -> Result<Checked, RecordingError> {
    check_with_hooks(store, manifest, Arc::new(Exts::new()), "")
}

/// [`check`], running these `ext` hooks. A recording made with other hooks
/// (another label, or none) is stale.
pub fn check_with_hooks(
    store: &Store,
    manifest: &RecordingManifest,
    exts: Arc<Exts>,
    hooks: &str,
) -> Result<Checked, RecordingError> {
    let b = bundle(store, manifest)?;
    let mut rt = Interp::from_bundle(&b)
        .map_err(RecordingError::Model)?
        .with_exts(exts);
    let input = InputLog::parse(&String::from_utf8_lossy(&store.get(&manifest.input_log)?))
        .map_err(|e| RecordingError::Corrupt(e.to_string()))?;
    let hashes = match manifest.frame_hashes {
        Some(h) => frame_hashes::decode(&store.get(&h)?)
            .map_err(|e| RecordingError::Corrupt(e.to_string()))?,
        None => return Err(RecordingError::Corrupt("no frame hashes".into())),
    };
    if hashes.len() as u64 != manifest.frames + 1 || input.frames.len() as u64 != manifest.frames {
        return Err(RecordingError::Corrupt(format!(
            "{} frame hashes and {} input frames for {} frames",
            hashes.len(),
            input.frames.len(),
            manifest.frames
        )));
    }
    // A recording with a trace hashes the view of its state; one without
    // hashes the canonical state.
    let encoder = match &manifest.trace {
        Some(t) => Some(
            Encoder::new(&FieldTable::decode(&store.get(&t.fields)?)?)
                .map_err(RecordingError::Model)?,
        ),
        None => None,
    };
    let hash_of = |rt: &Interp| -> Result<FrameHash, RecordingError> {
        match &encoder {
            Some(e) => Ok(FrameHash {
                state: Hash::of(
                    &e.encode(rt.spec(), rt.world())
                        .map_err(RecordingError::Model)?,
                ),
                ram: frame_hash(rt).ram,
            }),
            None => Ok(frame_hash(rt)),
        }
    };
    let start = Instant::now();
    let mut divergence = None;
    let mut event_writer = ModelEventWriter::new(
        manifest
            .events
            .as_ref()
            .map_or(gp_evidence::chunk::DEFAULT_CHUNK_FRAMES, |e| e.chunk_frames),
    );
    let mut event_chunks = Vec::new();
    event_chunks.extend(event_writer.push(&[]));
    if hash_of(&rt)? != hashes[0] {
        divergence = Some(Divergence::State(0));
    }
    for (i, f) in input.frames.iter().enumerate() {
        if divergence.is_some() {
            break;
        }
        let raised = rt.advance(&input_frame(f)).map_err(RecordingError::Model)?;
        event_chunks.extend(event_writer.push(&model_events(&rt, &raised)));
        if hash_of(&rt)? != hashes[i + 1] {
            divergence = Some(Divergence::State(i as u64 + 1));
        }
    }
    // The event stream must be what the replay raises.
    if divergence.is_none()
        && let Some(e) = &manifest.events
    {
        event_chunks.extend(event_writer.finish());
        let got: Vec<Hash> = event_chunks.iter().map(|c| Hash::of(c)).collect();
        if let Some(bad) = got
            .iter()
            .zip(&e.chunks)
            .position(|(a, b)| a != b)
            .or_else(|| (got.len() != e.chunks.len()).then_some(got.len().min(e.chunks.len())))
        {
            divergence = Some(Divergence::Events(bad as u64 * u64::from(e.chunk_frames)));
        }
    }
    let frames_per_second = input.frames.len() as f64 / start.elapsed().as_secs_f64().max(1e-9);
    let stale = (manifest.runtime != runtime_ref_with_hooks(hooks))
        .then(|| format!("{} {}", manifest.runtime.name, manifest.runtime.version));
    if divergence.is_none() && stale.is_none() {
        for cp in &manifest.checkpoints {
            let snap = store.get(&cp.snapshot)?;
            let Some(want) = hashes.get(cp.frame as usize) else {
                return Err(RecordingError::Corrupt(format!(
                    "a checkpoint at frame {} past the frame hashes",
                    cp.frame
                )));
            };
            let ok = gp_sim::World::from_canonical(rt.spec(), &snap)
                .is_ok_and(|w| Hash::from_bytes(w.hash()) == want.ram && w.canonical() == snap);
            if !ok {
                divergence = Some(Divergence::Checkpoint(cp.frame));
                break;
            }
        }
    }
    Ok(Checked {
        frames: manifest.frames,
        divergence,
        stale,
        frames_per_second,
    })
}
