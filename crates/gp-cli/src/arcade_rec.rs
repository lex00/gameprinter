//! Recordings of an arcade original (port wave 6): what gp-nes's
//! `recording` and `seek` do for the NES, on gp-arcade's Tapper, in the
//! same store objects, so `gp record`, `gp replay`, `gp query`, `gp
//! run-until`, `gp trace`, `gp events` and `gp recordings` read both.
//!
//! What differs from an NES recording:
//!
//! - The manifest's source is a romset (`romset`, with the set's identity
//!   and MAME name), and its runtime `gp-arcade`. Its `power_on` is the
//!   NES default, unread: the board powers on one way (the oracle's).
//! - The input log is an arcade log (`gp-arcade-input 0`), kept as given:
//!   it has no lag marks, so replay can't compare polling. The lag frames
//!   `gp record` reports come from the board's own input port reads.
//! - Each frame's state is [`gp_arcade::canonical`]'s recorded state: named
//!   fields, then the whole machine. Its hash is the frame hash's `state`,
//!   and the hash of the oracle's RAM record ([`gp_arcade::Tapper::ram_record`],
//!   the five RAMs) its `ram`.
//! - The event stream holds the board's events ([`gp_arcade::events`]):
//!   `irq` (the main CPU's CTC interrupts, with the handler and vector),
//!   `sound-irq`, `port-read`, `sound-latch`, `watchdog` and `ay-write`,
//!   counted in
//!   [`Geometry::MCR`]'s 481 lines of 512 dots.
//! - Checkpoints are the machine's snapshots. A recording from another
//!   build of gp-arcade (another [`gp_arcade::RUNTIME_VERSION`]) replays
//!   from power-on only, as gp-nes's do from another runtime.

use std::time::Instant;

use anyhow::{Result, bail};
use gp_arcade::events::{BoardEvent, Happened};
use gp_arcade::{Controls, Romset, Tapper};
use gp_evidence::chunk;
use gp_evidence::events::{self, Event, EventKind, EventWriter, Geometry};
use gp_evidence::frame_hashes::{self, FrameHash};
use gp_evidence::manifest::{
    Checkpoint, EventsRef, PowerOn, Provenance, RECORDING_SCHEMA, RecordingManifest, RuntimeRef,
    SourceRef, TraceRef,
};
use gp_evidence::recordings::{self, IndexEntry};
use gp_evidence::trace::{self, FieldDef, FieldTable, TraceWriter, Width};
use gp_evidence::{Hash, InputLog, Store};
use gp_nes::recording::{Divergence, DivergenceKind, Recorded, ReplayReport};

/// What an arcade recording holds besides the machine: bumped when its
/// recorded state or its events change. 2 added the interrupts (`irq`,
/// `sound-irq`) to the events; 3 keeps the snapshot's hash in the state
/// rather than the snapshot.
pub(crate) const RECORDING_VERSION: u32 = 3;

/// The runtime that makes arcade recordings: `gp-arcade`, the crate's
/// version with the board's, and [`RECORDING_VERSION`]
/// (`0.1.0 mcr 2 rec 3`).
pub(crate) fn runtime_ref() -> RuntimeRef {
    RuntimeRef {
        name: "gp-arcade".into(),
        version: format!(
            "{} rec {RECORDING_VERSION}",
            gp_arcade::RUNTIME_VERSION.trim_start_matches("gp-arcade ")
        ),
        commit: None,
    }
}

/// Whether a recording is of an arcade romset.
pub(crate) fn is_arcade(manifest: &RecordingManifest) -> bool {
    matches!(manifest.source, SourceRef::Romset { .. })
}

/// Where an arcade event stream counts its positions.
pub(crate) const GEOMETRY: Geometry = Geometry::MCR;

/// The field table of [`gp_arcade::canonical`]'s recorded state.
pub(crate) fn field_table() -> FieldTable {
    FieldTable {
        state_magic: gp_arcade::canonical::STATE_MAGIC,
        state_version: gp_arcade::canonical::STATE_VERSION,
        fields: gp_arcade::canonical::FIELDS
            .iter()
            .map(|(name, width)| FieldDef {
                name: (*name).to_owned(),
                width: width.map_or(Width::Region, Width::Fixed),
            })
            .collect(),
    }
}

/// The table a recording's states follow: its trace's, or this runtime's.
pub(crate) fn field_table_of(store: &Store, manifest: &RecordingManifest) -> Result<FieldTable> {
    Ok(match &manifest.trace {
        Some(t) => FieldTable::decode(&store.get(&t.fields)?)?,
        None => field_table(),
    })
}

/// A board event as a stored one.
pub(crate) fn stored(frame: u64, e: &BoardEvent) -> Event {
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

/// The frame hashes of a machine's state.
fn frame_hash(t: &Tapper) -> FrameHash {
    FrameHash {
        state: Hash::of(&t.recorded_state()),
        ram: Hash::of(&t.ram_record()),
    }
}

/// The controls of an input log entry, refusing what the board hasn't.
fn controls(f: &gp_evidence::input_log::InputFrame) -> Result<Controls> {
    let bits = f.buttons_word();
    if bits > 0xff {
        bail!(
            "buttons {bits:#x}: the arcade board reads bits 0 to 7 only (coin, start, right, left, down, up, tap, tilt)"
        );
    }
    if f.reset {
        bail!("the arcade board's reset isn't modelled");
    }
    Ok(Controls::from_bits(bits))
}

/// Refuses an input log the board can't play.
fn check_input(input: &InputLog) -> Result<()> {
    if input.frames.is_empty() {
        bail!("the input log is empty");
    }
    if input.arcade.is_none() {
        bail!(
            "an arcade romset plays an arcade input log (gp-arcade-input 0); this one is an NES log"
        );
    }
    Ok(())
}

/// What to record, as gp-nes's `RecordRequest` has it.
pub(crate) struct Request<'a> {
    pub set: &'a Romset,
    pub source_id: Option<String>,
    pub input: &'a InputLog,
    pub checkpoint_interval: u32,
    pub capture: gp_nes::recording::Capture,
    pub provenance: Provenance,
    pub name: String,
    pub created_at: String,
    pub notes: Option<String>,
}

/// Runs the set from power-on through the log and stores the recording.
pub(crate) fn record(store: &Store, req: Request<'_>) -> Result<Recorded> {
    check_input(req.input)?;
    let capture = req.capture;
    if capture.trace == Some(0) || capture.events == Some(0) {
        bail!("trace and event chunks must hold at least 1 frame");
    }
    if req.checkpoint_interval == 0 {
        bail!("the checkpoint interval must be at least 1 frame");
    }
    let interval = u64::from(req.checkpoint_interval);
    let mut t = Tapper::power_on(req.set);
    let mut hashes = Vec::with_capacity(req.input.frames.len() + 1);
    let mut checkpoints = Vec::new();
    let (mut bytes, mut new_bytes) = (0u64, 0u64);
    let mut put = |data: &[u8]| -> Result<Hash> {
        bytes += data.len() as u64;
        if !store.contains(&Hash::of(data)) {
            new_bytes += data.len() as u64;
        }
        Ok(store.put_bytes(data)?)
    };
    let (mut trace_bytes, mut events_bytes) = (0u64, 0u64);
    let mut trace = match capture.trace {
        Some(chunk_frames) => {
            let table = field_table().encode();
            trace_bytes += table.len() as u64;
            let fields = put(&table)?;
            Some((TraceWriter::new(chunk_frames, fields), fields, Vec::new()))
        }
        None => None,
    };
    let mut event_stream = capture.events.map(|chunk_frames| {
        (
            EventWriter::with_geometry(chunk_frames, GEOMETRY),
            Vec::new(),
        )
    });
    t.set_events(event_stream.is_some());
    let mut lag = 0usize;

    let start = Instant::now();
    for i in 0..=req.input.frames.len() {
        if i > 0 {
            t.step(controls(&req.input.frames[i - 1])?);
            lag += usize::from(!t.polled());
        }
        let state = t.recorded_state();
        hashes.push(FrameHash {
            state: Hash::of(&state),
            ram: Hash::of(&t.ram_record()),
        });
        if let Some((writer, _, chunks)) = &mut trace
            && let Some(c) = writer.push(&state)?
        {
            trace_bytes += c.len() as u64;
            chunks.push(put(&c)?);
        }
        if let Some((writer, chunks)) = &mut event_stream {
            let happened: Vec<Event> = t
                .take_events()
                .iter()
                .map(|e| stored(i as u64, e))
                .collect();
            if let Some(c) = writer.push(&happened) {
                events_bytes += c.len() as u64;
                chunks.push(put(&c)?);
            }
        }
        let n = i as u64;
        if n.is_multiple_of(interval) {
            checkpoints.push(Checkpoint {
                frame: n,
                snapshot: put(&t.snapshot())?,
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
        source: SourceRef::Romset {
            identity: req.set.identity.clone(),
            system: req.set.name.to_owned(),
        },
        runtime: runtime_ref(),
        power_on: PowerOn::default(),
        frames: req.input.frames.len() as u64,
        input_log: put(req.input.to_text().as_bytes())?,
        checkpoint_interval: req.checkpoint_interval,
        frame_hashes: Some(put(&frame_hashes::encode(&hashes))?),
        checkpoints,
        trace,
        events,
        provenance: req.provenance,
    };
    manifest.validate()?;
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
        lag_frames: lag,
        frames_per_second: req.input.frames.len() as f64 / elapsed.max(1e-9),
        manifest,
        bytes,
        new_bytes,
        trace_bytes,
        events_bytes,
        existing,
    })
}

/// The input log and frame hashes of a stored arcade recording.
pub(crate) fn load(
    store: &Store,
    manifest: &RecordingManifest,
) -> Result<(InputLog, Vec<FrameHash>)> {
    let text = String::from_utf8(store.get(&manifest.input_log)?)
        .map_err(|_| anyhow::anyhow!("the recording's input log isn't text"))?;
    let input = InputLog::parse(&text)?;
    check_input(&input)?;
    let Some(id) = manifest.frame_hashes else {
        bail!("the manifest has no frame hashes");
    };
    let hashes = frame_hashes::decode(&store.get(&id)?)?;
    if hashes.len() as u64 != manifest.frames + 1 || input.frames.len() as u64 != manifest.frames {
        bail!(
            "recording data is inconsistent: {} frames, {} input frames, {} frame hashes",
            manifest.frames,
            input.frames.len(),
            hashes.len()
        );
    }
    Ok((input, hashes))
}

/// What a divergence means on the arcade board, in place of gp-nes's
/// words for it.
pub(crate) fn describe(what: DivergenceKind) -> &'static str {
    match what {
        DivergenceKind::Ram => "RAM differs (game logic diverged)",
        DivergenceKind::State => "RAM matches, but the CPUs, timers, sound chips or picture differ",
        DivergenceKind::Events => "state matches, but the events within the frame differ",
        other => other.describe(),
    }
}

/// Whether this build made the recording, so its checkpoints restore.
pub(crate) fn made_by_this_runtime(manifest: &RecordingManifest) -> bool {
    manifest.runtime == runtime_ref()
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

/// Restores a checkpoint, refusing a snapshot that doesn't load.
fn restore(t: &mut Tapper, store: &Store, cp: &Checkpoint) -> Result<()> {
    t.restore(&store.get(&cp.snapshot)?)
        .map_err(|e| anyhow::anyhow!("the checkpoint at frame {}: {e}", cp.frame))
}

/// Every frame's recorded events, indexed by frame.
pub(crate) fn events_by_frame(
    store: &Store,
    manifest: &RecordingManifest,
) -> Result<Vec<Vec<Event>>> {
    let Some(stream) = &manifest.events else {
        bail!("the recording has no event stream");
    };
    let mut by_frame = Vec::new();
    for (i, id) in stream.chunks.iter().enumerate() {
        let expected = chunk::chunk_range(i, stream.chunk_frames, manifest.frames);
        let first = expected.start as usize;
        let c = events::decode_chunk_covering_in(&store.get(id)?, expected.clone(), GEOMETRY)?;
        by_frame.resize(first + (expected.end - expected.start) as usize, Vec::new());
        for e in c.events {
            by_frame[e.frame as usize].push(e);
        }
    }
    Ok(by_frame)
}

/// The recorded events of frames in `range`, or `None` without a stream.
pub(crate) fn stored_events(
    store: &Store,
    manifest: &RecordingManifest,
    range: std::ops::Range<u64>,
) -> Result<Option<Vec<Event>>> {
    let Some(stream) = &manifest.events else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for (i, id) in stream.chunks.iter().enumerate() {
        let covers = chunk::chunk_range(i, stream.chunk_frames, manifest.frames);
        if covers.end <= range.start || covers.start >= range.end {
            continue;
        }
        let c = events::decode_chunk_covering_in(&store.get(id)?, covers, GEOMETRY)?;
        out.extend(c.events.into_iter().filter(|e| range.contains(&e.frame)));
    }
    Ok(Some(out))
}

/// Replays from power-on, or from the checkpoint at `from_checkpoint`, and
/// compares every frame: RAM, then state, then events when it has them.
pub(crate) fn replay(
    store: &Store,
    manifest: &RecordingManifest,
    set: &Romset,
    from_checkpoint: Option<u64>,
) -> Result<ReplayReport> {
    let (input, hashes) = load(store, manifest)?;
    let mut t = Tapper::power_on(set);
    let recorded_events = match &manifest.events {
        Some(_) => Some(events_by_frame(store, manifest)?),
        None => None,
    };
    t.set_events(recorded_events.is_some());
    let from = match from_checkpoint {
        None | Some(0) => 0,
        Some(f) => {
            if !made_by_this_runtime(manifest) {
                bail!(
                    "this recording was made by {}, and this is {}. Its checkpoints are snapshots only the runtime that made them can restore. Replay it from power-on, or re-record it",
                    gp_nes::recording::describe_runtime(&manifest.runtime),
                    gp_nes::recording::describe_runtime(&runtime_ref())
                );
            }
            let Some(cp) = manifest.checkpoints.iter().find(|c| c.frame == f) else {
                bail!(
                    "no checkpoint at frame {f}; checkpoints are at frames {}",
                    manifest
                        .checkpoints
                        .iter()
                        .map(|c| c.frame.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            };
            restore(&mut t, store, cp)?;
            f
        }
    };
    let start = Instant::now();
    let report = |checked: u64, divergence| ReplayReport {
        from_frame: from,
        frames_checked: checked,
        frames_per_second: checked as f64 / start.elapsed().as_secs_f64().max(1e-9),
        divergence,
    };
    if let Some(what) = compare(&t, &hashes, from) {
        return Ok(report(0, Some(Divergence { frame: from, what })));
    }
    let mut checked = 0;
    for f in from..manifest.frames {
        t.step(controls(&input.frames[f as usize])?);
        checked += 1;
        let frame = f + 1;
        let happened: Vec<Event> = t.take_events().iter().map(|e| stored(frame, e)).collect();
        let what = compare(&t, &hashes, frame).or_else(|| {
            recorded_events
                .as_ref()
                .is_some_and(|by_frame| by_frame[frame as usize] != happened)
                .then_some(DivergenceKind::Events)
        });
        if let Some(what) = what {
            return Ok(report(checked, Some(Divergence { frame, what })));
        }
    }
    Ok(report(checked, None))
}

/// Restores every checkpoint and checks it against its frame's hashes, and
/// that it saves back to the same bytes. The first that doesn't, if any.
pub(crate) fn verify_checkpoints(
    store: &Store,
    manifest: &RecordingManifest,
    set: &Romset,
) -> Result<Option<Divergence>> {
    let (_, hashes) = load(store, manifest)?;
    let mut t = Tapper::power_on(set);
    for cp in &manifest.checkpoints {
        let snap = store.get(&cp.snapshot)?;
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

/// A machine positioned in an arcade recording, as gp-nes's seek cursor.
pub(crate) struct Cursor {
    tapper: Tapper,
    input: InputLog,
    hashes: Vec<FrameHash>,
    /// The checkpoint it started from, or `None` for power-on.
    pub checkpoint: Option<u64>,
    /// Frames replayed since the start.
    pub replayed: u64,
    /// Whether another runtime made the recording, so no checkpoint was used.
    pub stale: bool,
}

/// Opens a recording at `frame`, from the nearest checkpoint this runtime
/// may restore.
pub(crate) fn open(
    store: &Store,
    manifest: &RecordingManifest,
    set: &Romset,
    frame: u64,
) -> Result<Cursor> {
    if frame > manifest.frames {
        bail!(
            "frame {frame} is past the end of the recording, which has {} frames",
            manifest.frames
        );
    }
    let (input, hashes) = load(store, manifest)?;
    let mut tapper = Tapper::power_on(set);
    let usable = made_by_this_runtime(manifest);
    let checkpoint = manifest
        .checkpoints
        .iter()
        .filter(|c| usable && c.frame <= frame)
        .max_by_key(|c| c.frame);
    if let Some(cp) = checkpoint {
        restore(&mut tapper, store, cp)?;
    }
    let mut cursor = Cursor {
        tapper,
        input,
        hashes,
        checkpoint: checkpoint.map(|c| c.frame),
        replayed: 0,
        stale: !usable,
    };
    cursor.advance_to(frame)?;
    Ok(cursor)
}

impl Cursor {
    #[cfg(test)]
    pub fn tapper(&self) -> &Tapper {
        &self.tapper
    }

    pub fn into_tapper(self) -> Tapper {
        self.tapper
    }

    pub fn frame(&self) -> u64 {
        self.tapper.frame()
    }

    /// The input log entry applied to the state at `frame`.
    pub fn input(&self, frame: u64) -> Option<gp_evidence::input_log::InputFrame> {
        self.input.frames.get(frame as usize).copied()
    }

    pub fn hashes(&self) -> &[FrameHash] {
        &self.hashes
    }

    /// Runs the next frame of the recording.
    pub fn step(&mut self) -> Result<()> {
        let frame = self.frame();
        let Some(input) = self.input(frame) else {
            bail!(
                "frame {} is past the end of the recording, which has {} frames",
                frame + 1,
                self.input.frames.len()
            );
        };
        self.replayed += 1;
        self.tapper.step(controls(&input)?);
        Ok(())
    }

    pub fn advance_to(&mut self, frame: u64) -> Result<()> {
        if frame < self.frame() {
            bail!(
                "the cursor is at frame {} and only goes forward; open the recording again for frame {frame}",
                self.frame()
            );
        }
        while self.frame() < frame {
            self.step()?;
        }
        Ok(())
    }

    /// Whether the machine is in the state recorded for its frame.
    pub fn matches_recording(&self) -> bool {
        matches(&self.tapper, &self.hashes) != Some(false)
    }
}

/// Whether `t` is in the state `hashes` recorded for its frame, or `None`
/// past the last frame.
pub(crate) fn matches(t: &Tapper, hashes: &[FrameHash]) -> Option<bool> {
    let want = hashes.get(t.frame() as usize)?;
    Some(frame_hash(t) == *want)
}

/// The recorded state at `frame`, by replay.
pub(crate) fn replayed_state(
    store: &Store,
    manifest: &RecordingManifest,
    set: &Romset,
    frame: u64,
) -> Result<Vec<u8>> {
    Ok(open(store, manifest, set, frame)?.tapper.recorded_state())
}

/// The events of frames in `range`, by replay.
pub(crate) fn replayed_events(
    store: &Store,
    manifest: &RecordingManifest,
    set: &Romset,
    range: std::ops::Range<u64>,
) -> Result<Vec<Event>> {
    let end = range.end.min(manifest.frames + 1);
    let mut out = Vec::new();
    if range.start >= end {
        return Ok(out);
    }
    let mut cursor = open(store, manifest, set, range.start.saturating_sub(1))?;
    cursor.tapper.set_events(true);
    while cursor.frame() < end - 1 {
        cursor.step()?;
        let frame = cursor.frame();
        let happened = cursor.tapper.take_events();
        if range.contains(&frame) {
            out.extend(happened.iter().map(|e| stored(frame, e)));
        }
    }
    Ok(out)
}

/// What an instruction trace covered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Traced {
    pub instructions: u64,
    pub interrupts: u64,
}

/// `gp trace instructions` on an arcade recording: from the cursor's frame
/// to `last`, one line per instruction either Z80 starts, in time order
/// (a frame's two CPUs run in turns of a timeslice each; the lines are put
/// back in the order of their times), with `; frame N` before each frame
/// and `; IRQ` before an interrupt handler's first instruction:
///
/// ```text
/// ; frame 700, controls .......
/// M 02FD  3A 00 E0 FF                      AF:FFBA BC:0000 DE:0006 HL:E090 IX:E4A7 IY:E2E6 SP:E7EE LINE: 19,026 CYC:11666702
/// ; IRQ S vector FF
/// S 0038  F5 C5 D5 E5                      AF:...
/// ```
///
/// `M` is the main board's Z80 and `S` the sound board's. Each line is the
/// state before the instruction: its address, its bytes, the registers,
/// where the frame's beam was (line and dot from the frame's start, as the
/// events count them) and the CPU's cycles since power-on.
pub(crate) fn trace_instructions(
    cursor: &mut Cursor,
    last: u64,
    out: &mut impl std::io::Write,
) -> Result<Traced> {
    let mut traced = Traced::default();
    let columns = cursor
        .input
        .arcade
        .as_ref()
        .map_or_else(|| "CSRLDUT".to_owned(), |a| a.columns.clone());
    while cursor.frame() < last {
        let frame = cursor.frame() + 1;
        let held = cursor.input(frame - 1).map_or(0, |f| f.buttons_word());
        let letters: String = columns
            .chars()
            .enumerate()
            .map(|(i, c)| if held & (1 << i) != 0 { c } else { '.' })
            .collect();
        writeln!(out, "; frame {frame}, controls {letters}")?;
        cursor.tapper.set_trace(true);
        cursor.step()?;
        let mut entries = cursor.tapper.take_trace();
        cursor.tapper.set_trace(false);
        entries.sort_by_key(|e| e.time);
        let start = cursor.tapper.frame_start();
        let mut text = String::new();
        for e in &entries {
            if let Some(v) = e.interrupt {
                traced.interrupts += 1;
                text.push_str(&format!(
                    "; IRQ {} vector {v:02X}\n",
                    if e.cpu == 0 { 'M' } else { 'S' }
                ));
            } else {
                traced.instructions += 1;
                text.push_str(&trace_line(e, start));
                text.push('\n');
            }
        }
        out.write_all(text.as_bytes())?;
    }
    Ok(traced)
}

/// One instruction's line ([`trace_instructions`]).
pub(crate) fn trace_line(
    e: &gp_arcade::mcr::Trace,
    frame_start: gp_arcade::attotime::Attotime,
) -> String {
    let (line, dot) = gp_arcade::events::position(frame_start, e.time);
    let bytes = e
        .bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{} {:04X}  {bytes:<11}  {:<24} AF:{:04X} BC:{:04X} DE:{:04X} HL:{:04X} IX:{:04X} IY:{:04X} SP:{:04X} LINE:{line:>3},{dot:>3} CYC:{}",
        if e.cpu == 0 { 'M' } else { 'S' },
        e.pc,
        "",
        e.af,
        e.bc,
        e.de,
        e.hl,
        e.ix,
        e.iy,
        e.sp,
        e.cycles
    )
}

/// What `gp replay --all` found for one arcade recording.
pub(crate) fn check(
    store: &Store,
    manifest: &RecordingManifest,
    set: &Romset,
) -> Result<gp_nes::recording::Checked> {
    use gp_nes::recording::Checked;
    if !made_by_this_runtime(manifest) {
        let report = replay(store, manifest, set, None)?;
        return Ok(Checked::Stale {
            frames: report.frames_checked,
            divergence: report.divergence,
            runtime: gp_nes::recording::describe_runtime(&manifest.runtime),
        });
    }
    let mut report = replay(store, manifest, set, None)?;
    if report.divergence.is_none() {
        report.divergence = verify_checkpoints(store, manifest, set)?;
    }
    if report.divergence.is_none() && manifest.trace.is_some() {
        report.divergence = gp_nes::recording::verify_trace(store, manifest)?;
    }
    Ok(Checked::Replayed {
        frames: report.frames_checked,
        divergence: report.divergence,
        recorded_by: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("store")).unwrap();
        (dir, store)
    }

    /// A made-up log: idle, then tap held, then a coin.
    fn log(frames: usize) -> InputLog {
        let mut text = String::from("gp-arcade-input 0\nsystem test\ncolumns CSRLDUT\n");
        text.push_str(&format!("{} .......\n", frames / 2));
        text.push_str(&format!("{} ......T\n", frames - frames / 2 - 1));
        text.push_str("1 C......\n");
        InputLog::parse(&text).unwrap()
    }

    fn request<'a>(set: &'a Romset, input: &'a InputLog) -> Request<'a> {
        Request {
            set,
            source_id: None,
            input,
            checkpoint_interval: 10,
            capture: gp_nes::recording::Capture::ALL,
            provenance: Provenance::Scripted,
            name: "busy".into(),
            created_at: "2026-11-12T00:00:00Z".into(),
            notes: None,
        }
    }

    /// The made-up busy program, recorded and replayed: every frame
    /// matches, from power-on and from a checkpoint; the trace checks
    /// against the frame hashes; the events are the board's; a seek lands
    /// on the recorded state; and the same run gives the same ID.
    #[test]
    fn a_recording_replays_frame_exact() {
        let (_dir, store) = store();
        let set = gp_arcade::testrom::busy();
        let input = log(45);
        let rec = record(&store, request(&set, &input)).unwrap();
        assert_eq!(rec.manifest.frames, 45);
        assert_eq!(rec.manifest.checkpoints.len(), 5);
        assert!(matches!(rec.manifest.source, SourceRef::Romset { .. }));
        let r = replay(&store, &rec.manifest, &set, None).unwrap();
        assert_eq!((r.frames_checked, r.divergence), (45, None));
        let r = replay(&store, &rec.manifest, &set, Some(20)).unwrap();
        assert_eq!(
            (r.from_frame, r.frames_checked, r.divergence),
            (20, 25, None)
        );
        assert_eq!(
            gp_nes::recording::verify_trace(&store, &rec.manifest).unwrap(),
            None
        );
        assert_eq!(
            verify_checkpoints(&store, &rec.manifest, &set).unwrap(),
            None
        );
        let ev = stored_events(&store, &rec.manifest, 30..31)
            .unwrap()
            .unwrap();
        assert!(ev.iter().any(|e| e.kind == EventKind::AyWrite));
        assert_eq!(
            ev,
            replayed_events(&store, &rec.manifest, &set, 30..31).unwrap()
        );
        let traced = gp_nes::recording::trace_state(&store, &rec.manifest, 33)
            .unwrap()
            .unwrap();
        assert_eq!(
            traced,
            replayed_state(&store, &rec.manifest, &set, 33).unwrap()
        );
        let cursor = open(&store, &rec.manifest, &set, 33).unwrap();
        assert_eq!((cursor.checkpoint, cursor.replayed), (Some(30), 3));
        assert!(cursor.matches_recording());
        // The tap reached port 1 in the second half.
        assert_eq!(cursor.tapper().peek(0xe000), 0xef);
        let again = record(&store, request(&set, &input)).unwrap();
        assert_eq!(again.id, rec.id);
        assert!(again.existing);
    }

    /// The instruction trace of the busy program walks its loop on the
    /// main CPU and its own on the sound CPU, in time order, and changes
    /// nothing: the frame after it is still the recorded one.
    #[test]
    fn instructions_trace_both_cpus() {
        let (_dir, store) = store();
        let set = gp_arcade::testrom::busy();
        let input = log(10);
        let rec = record(&store, request(&set, &input)).unwrap();
        let mut cursor = open(&store, &rec.manifest, &set, 3).unwrap();
        let mut out = Vec::new();
        let traced = trace_instructions(&mut cursor, 4, &mut out).unwrap();
        assert!(cursor.matches_recording());
        assert_eq!(traced.interrupts, 0, "the busy program takes none");
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "; frame 4, controls .......");
        assert_eq!(traced.instructions as usize, lines.len() - 1);
        // The table jump `jp (hl)` is at 0029, and `in a,(01)` at 0003.
        assert!(text.contains("\nM 0029  E9 "), "{}", &text[..400]);
        assert!(text.contains("\nM 0003  DB 01 "));
        assert!(text.contains("\nS 0008  32 02 A0 "));
        let lines_at: Vec<(u16, u16)> = lines[1..]
            .iter()
            .map(|l| {
                let at = l.split("LINE:").nth(1).unwrap();
                let (a, b) = at.split_once(',').unwrap();
                let b = b.split_whitespace().next().unwrap();
                (a.trim().parse().unwrap(), b.parse().unwrap())
            })
            .collect();
        assert!(lines_at.windows(2).all(|w| w[0] <= w[1]), "time order");
    }

    /// Another program from the same log diverges, and replay says where.
    #[test]
    fn another_program_diverges() {
        let (_dir, store) = store();
        let set = gp_arcade::testrom::busy();
        let input = log(20);
        let rec = record(&store, request(&set, &input)).unwrap();
        let other = gp_arcade::testrom::looping();
        let r = replay(&store, &rec.manifest, &other, None).unwrap();
        assert_eq!(r.divergence.unwrap().frame, 1);
    }

    /// An NES log is refused rather than played as an arcade one.
    #[test]
    fn an_nes_log_is_refused() {
        let (_dir, store) = store();
        let set = gp_arcade::testrom::busy();
        let mut input = InputLog::new(1);
        input
            .frames
            .push(gp_evidence::input_log::InputFrame::default());
        let e = record(&store, request(&set, &input)).err().unwrap();
        assert!(e.to_string().contains("arcade input log"), "{e}");
    }

    /// With tapper.zip on this machine: thirty seconds of Tapper idling
    /// into its attract mode, recorded and replayed frame-exact, and each
    /// frame's RAM the board's own. Skipped without it.
    #[test]
    fn tapper_records_and_replays() {
        let Some(zip) = gp_arcade::romset::local_romset("tapper") else {
            eprintln!("skipped: no tapper romset");
            return;
        };
        let set = Romset::load_zip(&zip).unwrap();
        let (_dir, store) = store();
        let mut text = String::from("gp-arcade-input 0\nsystem tapper\ncolumns CSRLDUT\n");
        text.push_str("600 .......\n6 C......\n60 .......\n6 .S.....\n228 .......\n");
        let input = InputLog::parse(&text).unwrap();
        let rec = record(&store, request(&set, &input)).unwrap();
        let r = replay(&store, &rec.manifest, &set, None).unwrap();
        assert_eq!((r.frames_checked, r.divergence), (900, None));
        let mut t = Tapper::power_on(&set);
        let (_, hashes) = load(&store, &rec.manifest).unwrap();
        for (i, f) in input.frames.iter().enumerate() {
            t.step(controls(f).unwrap());
            assert_eq!(
                hashes[i + 1].ram,
                Hash::of(&t.ram_record()),
                "frame {}",
                i + 1
            );
        }
    }
}
