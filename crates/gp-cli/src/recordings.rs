//! Recordings of the original and what reads them: `gp record`, `gp
//! replay`, `gp query`, `gp run-until`, `gp trace frame|diff|fields|verify|
//! instructions`, `gp events`, `gp recordings list|show` and `gp movie`
//! (docs/recording.md, docs/movies.md). Brought from game-printer's main.rs
//! with their flags and outputs; what gameprinter's layout changed:
//!
//! - ROMs named by `--source` are the ROM entries of the games' roms.toml
//!   files, not the old repo's sources/ folder;
//! - `recordings list` and `show` scan the recordings log (crate::recindex)
//!   instead of keeping gp-index's SQLite file;
//! - no run recorder (gp-metrics): the commands keep no metrics;
//! - `record --model` and the replay of a model's recordings are not here
//!   yet: the new world runs a model with hooks through its game's module.
//!
//! These read gp-nes recordings, so an arcade original is refused in words.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use gp_evidence::{Hash, InputLog, Paths, Store};
use serde_json::{Value, json};

use crate::Outcome;
use crate::open::{Opened, RomFolder, now_rfc3339};
use crate::recindex::Index;

/// `gp record`'s arguments.
#[derive(clap::Args)]
pub struct RecordArgs {
    /// The source to run, by the ROM entry's ID in a game's roms.toml
    /// (e.g. donkey-kong-oe); found in $GP_ROM_DIR by hash.
    #[arg(long, conflicts_with = "rom", required_unless_present = "rom")]
    source: Option<String>,
    /// A ROM file to run instead of a registered source.
    #[arg(long)]
    rom: Option<PathBuf>,
    /// The input log to play.
    #[arg(long)]
    input: PathBuf,
    /// A short name for the recording, kept in the index (default: the
    /// input file's name). Recording the same run under a new name adds an
    /// index entry with the same ID.
    #[arg(long)]
    name: Option<String>,
    /// Free-text notes, kept in the index.
    #[arg(long)]
    notes: Option<String>,
    /// Frames between checkpoints.
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u32).range(1..))]
    checkpoint_interval: u32,
    /// Don't store the frame trace (every frame's full state).
    #[arg(long)]
    no_trace: bool,
    /// Don't store the event stream.
    #[arg(long)]
    no_events: bool,
    /// Power-on RAM: zeros, ones, or fceux (FCEUX's default fill of
    /// internal RAM, 00 00 00 00 FF FF FF FF repeated) (the manifest's
    /// power_on.ram_init).
    #[arg(long, default_value = "zeros", value_parser = ["zeros", "ones", "fceux"])]
    ram_init: String,
    /// Console region: ntsc or pal (the manifest's power_on.region).
    #[arg(long, default_value = "ntsc", value_parser = ["ntsc", "pal"])]
    region: String,
    /// Where the input came from (the manifest's provenance): `scripted`,
    /// or `human-play` for a person's play session.
    #[arg(long, default_value = "scripted", value_parser = ["scripted", "human-play"])]
    provenance: String,
}

/// `gp replay`'s arguments.
#[derive(clap::Args)]
pub struct ReplayArgs {
    /// The recording's ID, or a unique prefix of at least 4 characters.
    #[arg(required_unless_present = "all")]
    id: Option<String>,
    /// Replay every recording in the store whose ROM is in $GP_ROM_DIR,
    /// and restore each of its checkpoints. Recordings made by another
    /// runtime are reported as stale and don't fail the check. Retired
    /// recordings are skipped.
    #[arg(long, conflicts_with_all = ["id", "from_checkpoint", "rom"])]
    all: bool,
    /// With `--all`, replay retired recordings too.
    #[arg(long, requires = "all")]
    retired: bool,
    /// Start from the checkpoint at this frame instead of power-on. Only
    /// for recordings made by this runtime.
    #[arg(long)]
    from_checkpoint: Option<u64>,
    /// The ROM file, instead of finding it in $GP_ROM_DIR by hash.
    #[arg(long)]
    rom: Option<PathBuf>,
}

/// `gp query`'s arguments.
#[derive(clap::Args)]
pub struct QueryArgs {
    /// The recording's ID, or a unique prefix of at least 4 characters.
    id: String,
    /// The frame: the state after this many frames, 0 for power-on.
    #[arg(long)]
    frame: u64,
    /// What to read: `ram[0x0770]`, `ram[0x0700..0x0710]`, `oam[*]`,
    /// `cpu.pc`, `cpu` (every CPU field), `bus[0xfffc]`, `frame`. Quote
    /// each one: zsh treats an unquoted `ram[0x0770]` as a glob pattern.
    #[arg(required = true)]
    selectors: Vec<String>,
    /// The ROM file, instead of finding it in $GP_ROM_DIR by hash.
    #[arg(long)]
    rom: Option<PathBuf>,
    /// Restore a checkpoint and replay even when the trace has the frame.
    #[arg(long)]
    replay: bool,
}

/// `gp run-until`'s arguments.
#[derive(clap::Args)]
pub struct RunUntilArgs {
    /// The recording's ID, or a unique prefix of at least 4 characters.
    id: String,
    /// The condition, checked at the starting frame and after each frame.
    /// Quote it, as in `'ram[0x0770] == 1'`.
    condition: String,
    /// The frame to start from.
    #[arg(long, default_value_t = 0)]
    from: u64,
    /// Frames to run at most (default: to the end of the recording).
    /// Past the end of the input log, its last input stays held.
    #[arg(long)]
    max_frames: Option<u32>,
    /// Selectors to read at the frame where it stopped, quoted.
    #[arg(long = "show", value_name = "SELECTOR")]
    show: Vec<String>,
    /// The ROM file, instead of finding it in $GP_ROM_DIR by hash.
    #[arg(long)]
    rom: Option<PathBuf>,
    /// Restore a checkpoint and replay even when the trace has the frames.
    #[arg(long)]
    replay: bool,
}

/// `gp events`' arguments.
#[derive(clap::Args)]
pub struct EventsArgs {
    /// The recording's ID, or a unique prefix of at least 4 characters.
    id: String,
    /// Which frames: `N`, `A..B` (B excluded), `A..=B` or `A..`.
    #[arg(long, default_value = "1..")]
    frames: String,
    /// Only these kinds, comma-separated: nmi, irq, sprite0-hit,
    /// bank-switch, controller-read, controller-write, ppu-write, apu-write.
    #[arg(long, value_delimiter = ',')]
    kind: Vec<String>,
    /// The ROM file, for a recording without an event stream, instead of
    /// finding it in $GP_ROM_DIR by hash.
    #[arg(long)]
    rom: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum TraceCmd {
    /// Print the state at one frame, by field. Read from the recording's
    /// trace, or by restoring a checkpoint and replaying when it has none.
    Frame {
        /// The recording's ID, or a unique prefix of at least 4 characters.
        id: String,
        /// The frame: 0 is power-on, the last is the recording's frame count.
        frame: u64,
        /// Only these fields, comma-separated. A name selects that field and
        /// every field under it: `cpu`, `ppu`, `apu.pulse1`, `ram`, `oam`.
        /// Without it, fields over 32 bytes are summarized.
        #[arg(long, value_delimiter = ',')]
        field: Vec<String>,
        /// The ROM file, for a recording without a trace, instead of finding
        /// it in $GP_ROM_DIR by hash.
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// Compare two recordings frame by frame and report the first frame
    /// whose state differs, with the fields that differ. Exits 1 if they
    /// differ.
    Diff {
        /// The first recording's ID or prefix.
        a: String,
        /// The second recording's ID or prefix.
        b: String,
    },
    /// Print the field table a recording's states follow: each field's
    /// name, kind and width, in order.
    Fields {
        /// The recording's ID, or a unique prefix of at least 4 characters.
        id: String,
    },
    /// Check a recording's trace against its frame hashes. Exits 1 if a
    /// frame doesn't match.
    Verify {
        /// The recording's ID, or a unique prefix of at least 4 characters.
        id: String,
    },
    /// Print one line per CPU instruction over a window of frames, in the
    /// layout of nestest.log, by restoring the nearest checkpoint and
    /// replaying up to the window.
    Instructions {
        /// The recording's ID, or a unique prefix of at least 4 characters.
        id: String,
        /// The frames to trace: `100..102` is frames 100 and 101, `100..=102`
        /// adds 102, `100..` runs to the last frame, `100` is one frame.
        /// Frame N is the one that produces the state at frame N, run with
        /// input log entry N-1; `gp events` numbers frames the same way.
        #[arg(long)]
        frames: String,
        /// Write the trace to this file instead of standard output.
        #[arg(long)]
        out: Option<PathBuf>,
        /// The ROM file, instead of finding it in $GP_ROM_DIR by hash.
        #[arg(long)]
        rom: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum RecordingsCmd {
    /// Every recording in the store, one line per name, oldest first. Reads
    /// the recordings log and the manifests it names.
    List {
        /// Only recordings of this source: a ROM entry's ID, or a prefix of
        /// the ROM's SHA-256.
        #[arg(long)]
        source: Option<String>,
        /// Only recordings whose runtime name and version contain this text, such as `gp-nes 2`.
        #[arg(long)]
        runtime: Option<String>,
        /// Only recordings made by a runtime other than this one.
        #[arg(long)]
        stale: bool,
        /// Only golden recordings.
        #[arg(long)]
        golden: bool,
        /// Include retired recordings, which are left out otherwise.
        #[arg(long, conflicts_with = "only_retired")]
        retired: bool,
        /// Only retired recordings.
        #[arg(long)]
        only_retired: bool,
    },
    /// Show a recording's names and manifest summary, and the store objects
    /// it uses with their sizes.
    Show {
        /// The recording's ID, or a unique prefix of at least 4 characters.
        id: String,
    },
}

pub fn record_cmd(a: &RecordArgs, paths: &Paths) -> Result<Outcome> {
    record(
        a.source.as_deref(),
        a.rom.as_deref(),
        &a.input,
        IndexLabel {
            name: a.name.as_deref(),
            notes: a.notes.as_deref(),
        },
        Keep {
            checkpoint_interval: a.checkpoint_interval,
            capture: gp_nes::recording::Capture {
                trace: (!a.no_trace).then_some(gp_evidence::chunk::DEFAULT_CHUNK_FRAMES),
                events: (!a.no_events).then_some(gp_evidence::chunk::DEFAULT_CHUNK_FRAMES),
            },
            power_on: power_on(&a.ram_init, &a.region),
        },
        if a.provenance == "human-play" {
            gp_evidence::manifest::Provenance::HumanPlay
        } else {
            gp_evidence::manifest::Provenance::Scripted
        },
        paths,
    )
}

pub fn replay_cmd(a: &ReplayArgs, paths: &Paths) -> Result<Outcome> {
    if a.all {
        return replay_all(paths, a.retired);
    }
    replay(
        a.id.as_deref().expect("clap requires an id"),
        a.from_checkpoint,
        a.rom.as_deref(),
        paths,
    )
}

pub fn query_cmd(a: &QueryArgs, paths: &Paths) -> Result<Outcome> {
    query(
        &a.id,
        a.frame,
        &a.selectors,
        (a.rom.as_deref(), a.replay),
        paths,
    )
}

pub fn run_until_cmd(a: &RunUntilArgs, paths: &Paths) -> Result<Outcome> {
    run_until(
        &a.id,
        &a.condition,
        a.from,
        a.max_frames,
        &a.show,
        (a.rom.as_deref(), a.replay),
        paths,
    )
}

pub fn events_cmd(a: &EventsArgs, paths: &Paths) -> Result<Outcome> {
    events(&a.id, &a.frames, &a.kind, a.rom.as_deref(), paths)
}

pub fn trace_cmd(cmd: &TraceCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        TraceCmd::Instructions {
            id,
            frames,
            out,
            rom,
        } => trace_instructions(id, frames, out.as_deref(), rom.as_deref(), paths),
        TraceCmd::Frame {
            id,
            frame,
            field,
            rom,
        } => trace_frame(id, *frame, field, rom.as_deref(), paths),
        TraceCmd::Diff { a, b } => trace_diff(a, b, paths),
        TraceCmd::Fields { id } => trace_fields(&Opened::open(paths, id)?),
        TraceCmd::Verify { id } => trace_verify(id, paths),
    }
}

pub fn recordings_cmd(cmd: &RecordingsCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        RecordingsCmd::List {
            source,
            runtime,
            stale,
            golden,
            retired,
            only_retired,
        } => recordings_list(
            crate::recindex::Filter {
                source: source.clone(),
                runtime: runtime.clone(),
                stale_against: stale.then(gp_nes::runtime_ref),
                golden: *golden,
                retired: match (*retired, *only_retired) {
                    (_, true) => crate::recindex::Retired::Only,
                    (true, false) => crate::recindex::Retired::Include,
                    (false, false) => crate::recindex::Retired::Skip,
                },
            },
            paths,
        ),
        RecordingsCmd::Show { id } => recordings_show(id, paths),
    }
}

/// A recording's result in `replay --all` that this tool can't check: a
/// run of a model in the old tool's interpreter.
struct ModelRun;

/// Refuses a model recording, which the old tool read through its
/// interpreter and gameprinter doesn't record yet.
fn refuse_model(opened: &Opened) -> Result<()> {
    if let gp_evidence::manifest::SourceRef::Model { name, .. } = &opened.manifest.source {
        bail!(
            "{} is a run of the model `{name}`, which gameprinter can't read yet: it reads ROM recordings",
            opened.short()
        );
    }
    Ok(())
}

/// How `gp record` powers on, and what it keeps besides the input log and
/// frame hashes.
struct Keep {
    checkpoint_interval: u32,
    capture: gp_nes::recording::Capture,
    /// The power-on state the recording runs (`--ram-init`, `--region`).
    power_on: gp_evidence::manifest::PowerOn,
}

/// What `gp record` puts in the index entry and not in the manifest.
pub(crate) struct IndexLabel<'a> {
    pub name: Option<&'a str>,
    pub notes: Option<&'a str>,
}

/// The power-on state `gp record` runs: the defaults, with the RAM init and
/// region its options give (clap allows only the values matched here).
fn power_on(ram_init: &str, region: &str) -> gp_evidence::manifest::PowerOn {
    use gp_evidence::manifest::{PowerOn, RamInit, Region};
    PowerOn {
        ram_init: match ram_init {
            "ones" => RamInit::Ones,
            "fceux" => RamInit::Fceux,
            _ => RamInit::Zeros,
        },
        region: if region == "pal" {
            Region::Pal
        } else {
            Region::Ntsc
        },
        ..PowerOn::default()
    }
}

fn record(
    source: Option<&str>,
    rom: Option<&Path>,
    input_path: &Path,
    label: IndexLabel<'_>,
    Keep {
        checkpoint_interval,
        capture,
        power_on,
    }: Keep,
    provenance: gp_evidence::manifest::Provenance,
    paths: &Paths,
) -> Result<Outcome> {
    let found = crate::open::rom_for_source(source, rom, paths)?;
    let text = fs::read_to_string(input_path)
        .with_context(|| format!("reading {}", input_path.display()))?;
    let input = InputLog::parse(&text)?;
    let store = Store::open(&paths.store)?;
    let name = label
        .name
        .map(str::to_owned)
        .or_else(|| {
            input_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "recording".into());

    let recorded = {
        gp_nes::recording::record_with(
            &store,
            gp_nes::recording::RecordRequest {
                rom: &found.bytes,
                source: gp_evidence::manifest::SourceRef::NesRom {
                    sha256: found.image.sha256.clone(),
                    mapper: found.image.header.mapper,
                },
                source_id: found.source_id.clone(),
                power_on,
                input: &input,
                checkpoint_interval,
                provenance,
                name: name.clone(),
                created_at: now_rfc3339(),
                notes: label.notes.map(str::to_owned),
            },
            capture,
        )?
    };
    let mut text = format!(
        "recorded {} frames of {} ({} lag) at {:.0} frames/s\nid: {}\n{} bytes ({} new to the store): {} checkpoints, every {} frames",
        recorded.manifest.frames,
        found.source_id.as_deref().unwrap_or("an unregistered ROM"),
        recorded.lag_frames,
        recorded.frames_per_second,
        recorded.id,
        recorded.bytes,
        recorded.new_bytes,
        recorded.manifest.checkpoints.len(),
        recorded.manifest.checkpoint_interval
    );
    let kept: Vec<String> = [
        recorded
            .manifest
            .trace
            .as_ref()
            .map(|_| format!("trace {} bytes", recorded.trace_bytes)),
        recorded
            .manifest
            .events
            .as_ref()
            .map(|_| format!("events {} bytes", recorded.events_bytes)),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !kept.is_empty() {
        text.push_str(&format!(", {}", kept.join(", ")));
    }
    if recorded.existing {
        text.push_str("\nthe store already had this run, so the ID is the same as before");
    }
    // No SQLite index to bring up to date: the log is the index.
    let indexed = true;
    Ok(Outcome::ok(
        json!({
            "id": recorded.id,
            "existing": recorded.existing,
            "frames": recorded.manifest.frames,
            "lag_frames": recorded.lag_frames,
            "fps": recorded.frames_per_second,
            "bytes": recorded.bytes,
            "new_bytes": recorded.new_bytes,
            "trace_bytes": recorded.manifest.trace.as_ref().map(|_| recorded.trace_bytes),
            "events_bytes": recorded.manifest.events.as_ref().map(|_| recorded.events_bytes),
            "source": found.source_id,
            "indexed": indexed,
        }),
        text,
    ))
}

fn replay(
    id: &str,
    from_checkpoint: Option<u64>,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let opened = Opened::open(paths, id)?;
    if matches!(
        opened.manifest.source,
        gp_evidence::manifest::SourceRef::Model { .. }
    ) {
        if from_checkpoint.is_some() || rom.is_some() {
            bail!("a model recording replays from power-on, with no ROM");
        }
        bail!(
            "{} is a model recording, which gameprinter can't replay yet",
            opened.short()
        );
    }
    let found = opened.rom(rom, paths)?;
    let Opened {
        store,
        entry,
        manifest,
    } = opened;
    // A recording from another runtime replays from power-on only; replay
    // refuses its checkpoints. One from a runtime that differs only in its
    // fork patch keeps them when they round-trip: the one asked for, which
    // replay refuses otherwise, or for a whole replay every one of them.
    let patch_only = gp_nes::recording::lineage(&manifest) == gp_nes::recording::Lineage::PatchOnly;
    let stale = match from_checkpoint {
        Some(f) if f > 0 && patch_only => false,
        _ => !gp_nes::recording::keeps_checkpoints(&store, &manifest, &found.bytes)?,
    };
    let runtimes = format!(
        "Recorded by {}; this is {}",
        gp_nes::recording::describe_runtime(&manifest.runtime),
        gp_nes::recording::describe_runtime(&gp_nes::runtime_ref())
    );
    let mut report =
        { gp_nes::recording::replay(&store, &manifest, &found.bytes, from_checkpoint)? };
    // A whole replay also checks the trace against the frame hashes, as
    // `gp replay --all` does. It needs no runtime, so a stale recording's
    // trace is checked too, and damage there comes before staleness.
    let whole = from_checkpoint.unwrap_or(0) == 0;
    if whole && manifest.trace.is_some() {
        let trace = gp_nes::recording::verify_trace(&store, &manifest)?;
        report.divergence = match (report.divergence, trace) {
            (Some(replayed), _) if !stale => Some(replayed),
            (replayed, trace) => trace.or(replayed),
        };
    }
    let trace_damaged = report
        .divergence
        .as_ref()
        .is_some_and(|d| d.what == gp_nes::recording::DivergenceKind::Trace);
    // A patch may add an event hook, so for a patch-only recording differing
    // events alone are a runtime change, as `gp replay --all` counts them.
    let stale = stale
        || (patch_only
            && report
                .divergence
                .as_ref()
                .is_some_and(|d| d.what == gp_nes::recording::DivergenceKind::Events));
    let stale_report = stale && !trace_damaged;
    let span = format!("frames {} to {}", report.from_frame, manifest.frames);
    let (mut text, exit) = match &report.divergence {
        None => (
            format!(
                "replayed {} ({}) {span}: every frame matches, at {:.0} frames/s",
                entry.name,
                &entry.id.to_hex()[..12],
                report.frames_per_second
            ),
            0,
        ),
        Some(d) if stale_report => (
            format!(
                "replayed {} ({}) {span}: STALE, differs from frame {} ({})",
                entry.name,
                &entry.id.to_hex()[..12],
                d.frame,
                d.what.describe()
            ),
            1,
        ),
        Some(d) => (
            format!(
                "replayed {} ({}) {span}: DIVERGED at frame {}: {}",
                entry.name,
                &entry.id.to_hex()[..12],
                d.frame,
                d.what.describe()
            ),
            1,
        ),
    };
    if stale {
        text.push_str(&format!("\n{runtimes}."));
        text.push_str(if trace_damaged {
            "\nThe stored trace is damaged, which no runtime change explains: `gp trace verify` checks it."
        } else if report.divergence.is_some() {
            "\nExpect differences after a runtime change: re-record it rather than investigating (docs/recording.md)."
        } else {
            "\nThis runtime reproduces it, but can't check its checkpoints: re-record it to replace them (docs/recording.md)."
        });
    }
    Ok(Outcome {
        json: json!({
            "id": entry.id,
            "from_frame": report.from_frame,
            "frames_checked": report.frames_checked,
            "fps": report.frames_per_second,
            "divergence": report.divergence.as_ref().map(|d| json!({ "frame": d.frame, "what": format!("{:?}", d.what).to_lowercase() })),
            "stale": stale,
            "recorded_runtime": manifest.runtime,
        }),
        text,
        exit,
    })
}

/// Opens a recording at `frame`, timed as the `seek` metrics.
fn seek(opened: &Opened, rom: &[u8], frame: u64) -> Result<gp_nes::seek::Cursor> {
    let cursor = { gp_nes::seek::open(&opened.store, &opened.manifest, rom, frame)? };
    Ok(cursor)
}

/// A note for a state that isn't the one recorded, or `None` if it is.
fn mismatch_note(cursor: &gp_nes::seek::Cursor, opened: &Opened) -> Result<Option<String>> {
    let matches = cursor.matches_recording()?;
    Ok(note_unless(matches, cursor.frame(), cursor.stale, opened))
}

/// The note for a state at `frame` that doesn't match the recording, or
/// `None` if `matches`.
fn note_unless(matches: bool, frame: u64, stale: bool, opened: &Opened) -> Option<String> {
    if matches {
        return None;
    }
    Some(if stale {
        format!(
            "frame {frame} doesn't match the recording, which {} made; this is {}. Re-record it (docs/recording.md)",
            gp_nes::recording::describe_runtime(&opened.manifest.runtime),
            gp_nes::recording::describe_runtime(&gp_nes::runtime_ref())
        )
    } else {
        format!(
            "frame {frame} doesn't match the recording: replay diverged (gp replay finds where)"
        )
    })
}

/// `path = 113 (0x71)`.
pub(crate) fn entry_line(e: &gp_runtime::Entry) -> String {
    match e.value {
        gp_runtime::Value::Int(v) if v >= 0 => {
            let width = if v < 0x100 { 4 } else { 6 };
            format!("{} = {v} ({v:#0width$x})", e.path)
        }
        ref v => format!("{} = {v}", e.path),
    }
}

pub(crate) fn entry_json(e: &gp_runtime::Entry) -> Value {
    let value = match &e.value {
        gp_runtime::Value::Int(v) | gp_runtime::Value::Fixed(v) => json!(v),
        gp_runtime::Value::Flag(b) => json!(b),
        other => json!(other.to_string()),
    };
    json!({ "path": e.path, "value": value })
}

fn query(
    id: &str,
    frame: u64,
    selectors: &[String],
    (rom, replay): (Option<&Path>, bool),
    paths: &Paths,
) -> Result<Outcome> {
    use gp_runtime::Runtime as _;
    let opened = Opened::open(paths, id)?;
    refuse_model(&opened)?;
    // A --rom given is checked even when the trace answers.
    let given = rom.map(|_| opened.rom(rom, paths)).transpose()?;
    if !replay && let Some(out) = crate::from_trace::query(&opened, frame, selectors)? {
        return Ok(out);
    }
    let rom = match given {
        Some(found) => found.bytes,
        None => opened.rom(None, paths)?.bytes,
    };
    let cursor = seek(&opened, &rom, frame)?;
    let note = mismatch_note(&cursor, &opened)?;
    let (checkpoint, replayed, stale) = (cursor.checkpoint, cursor.replayed, cursor.stale);
    let runtime = gp_nes::protocol::NesRuntime::new(cursor.into_nes())?;
    let mut entries = Vec::new();
    for s in selectors {
        entries.extend(runtime.query(s).map_err(anyhow::Error::msg)?);
    }
    let mut text: Vec<String> = entries.iter().map(entry_line).collect();
    if let Some(note) = &note {
        text.push(format!("note: {note}"));
    }
    Ok(Outcome {
        json: json!({
            "id": opened.entry.id,
            "frame": frame,
            "read_from": "replay",
            "checkpoint": checkpoint,
            "replayed": replayed,
            "stale": stale,
            "matches_recording": note.is_none(),
            "entries": entries.iter().map(entry_json).collect::<Vec<_>>(),
        }),
        text: text.join("\n"),
        exit: u8::from(note.is_some()),
    })
}

fn run_until(
    id: &str,
    condition: &str,
    from: u64,
    max_frames: Option<u32>,
    show: &[String],
    (rom, replay): (Option<&Path>, bool),
    paths: &Paths,
) -> Result<Outcome> {
    use gp_runtime::Runtime as _;
    // Refuse a bad condition before replaying anything.
    gp_runtime::select::Condition::parse(condition).map_err(anyhow::Error::msg)?;
    let opened = Opened::open(paths, id)?;
    // A --rom given is checked even when the trace answers.
    let given = rom.map(|_| opened.rom(rom, paths)).transpose()?;
    if !replay
        && let Some(out) = crate::from_trace::run_until(&opened, condition, from, max_frames, show)?
    {
        return Ok(out);
    }
    let rom = match given {
        Some(found) => found.bytes,
        None => opened.rom(None, paths)?.bytes,
    };
    let cursor = seek(&opened, &rom, from)?;
    let frames = opened.manifest.frames;
    let max = max_frames.unwrap_or(u32::try_from(frames - from).unwrap_or(u32::MAX));
    // The log's entries from here on. Past its end the protocol holds the
    // last input it was given, so a start at the end gets the log's last.
    let first = if from < frames {
        from
    } else {
        frames.saturating_sub(1)
    };
    // Starting at the end holds the log's last entry, whose reset (if any)
    // already happened, so it isn't pressed again.
    let inputs: Vec<gp_runtime::InputFrame> = (first..frames)
        .filter_map(|f| cursor.input(f))
        .map(|f| gp_runtime::InputFrame {
            reset: f.reset && from < frames,
            ..gp_runtime::InputFrame::pads(f.pads)
        })
        .collect();
    let (hashes, stale) = (cursor.hashes().to_vec(), cursor.stale);
    let mut runtime = gp_nes::protocol::NesRuntime::new(cursor.into_nes())?;
    // Only the frame it stops at matters here, so no events are kept: a
    // long run would otherwise hold every frame's events.
    let met = {
        runtime
            .run_until_frames(condition, &inputs, max)
            .map_err(anyhow::Error::msg)?
    };
    // Frames run, counted from the frame number.
    let ran = runtime.frame() - from;
    let at = runtime.frame();
    // Past the last frame nothing was recorded to compare with.
    let matches = gp_nes::seek::matches(runtime.nes(), &hashes)?;
    let note = note_unless(matches != Some(false), at, stale, &opened);
    let mut entries = Vec::new();
    for s in show {
        entries.extend(runtime.query(s).map_err(anyhow::Error::msg)?);
    }
    let mut text = vec![if met {
        format!("{condition}: holds at frame {at} ({ran} frames from frame {from})")
    } else {
        format!("{condition}: doesn't hold by frame {at} ({ran} frames from frame {from})")
    }];
    text.extend(entries.iter().map(entry_line));
    if let Some(note) = &note {
        text.push(format!("note: {note}"));
    }
    Ok(Outcome {
        json: json!({
            "id": opened.entry.id,
            "condition": condition,
            "met": met,
            "frame": at,
            "from": from,
            "frames_run": ran,
            "read_from": "replay",
            "matches_recording": matches,
            "entries": entries.iter().map(entry_json).collect::<Vec<_>>(),
        }),
        text: text.join("\n"),
        exit: u8::from(!met || note.is_some()),
    })
}

/// Parses `--frames` for a recording whose last frame is `last`: `N`,
/// `A..B` (B excluded), `A..=B`, or `A..` (to the last frame). An empty or
/// reversed range, or one past the last frame, is an error.
pub(crate) fn parse_frames(spec: &str, last: u64) -> Result<std::ops::Range<u64>> {
    let num = |s: &str| -> Result<u64> {
        s.trim()
            .parse()
            .with_context(|| format!("{s:?} in --frames {spec} isn't a frame number"))
    };
    let past = || anyhow::anyhow!("--frames {spec} goes past the recording's last frame, {last}");
    let after = |n: u64| n.checked_add(1).ok_or_else(past);
    let (start, end) = if let Some((a, b)) = spec.split_once("..=") {
        (num(a)?, after(num(b)?)?)
    } else if let Some((a, b)) = spec.split_once("..") {
        let end = if b.trim().is_empty() {
            after(last)?
        } else {
            num(b)?
        };
        (num(a)?, end)
    } else {
        let n = num(spec)?;
        (n, after(n)?)
    };
    if end <= start {
        bail!("--frames {spec} is empty: a range ends after it starts");
    }
    if end - 1 > last {
        return Err(past());
    }
    Ok(start..end)
}

fn trace_instructions(
    id: &str,
    frames: &str,
    out: Option<&Path>,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let opened = Opened::open(paths, id)?;
    let rom = opened.rom(rom, paths)?.bytes;
    let range = parse_frames(frames, opened.manifest.frames)?;
    // Frame 0 is power-on, which no instructions produce.
    let (start, end) = (range.start.max(1), range.end);
    if start >= end {
        bail!(
            "frame 0 is the power-on state, which no instructions produce; traces start at frame 1"
        );
    }
    // Frame `start` runs from the state before it.
    let mut cursor = seek(&opened, &rom, start - 1)?;
    let mut buffer = Vec::new();
    let traced = {
        match out {
            Some(path) => {
                let file = fs::File::create(path)
                    .with_context(|| format!("creating {}", path.display()))?;
                let mut file = std::io::BufWriter::new(file);
                let traced = gp_nes::instructions::trace(&mut cursor, end - 1, &mut file)?;
                file.flush()?;
                traced
            }
            None => gp_nes::instructions::trace(&mut cursor, end - 1, &mut buffer)?,
        }
    };
    let note = mismatch_note(&cursor, &opened)?;
    if let Some(note) = &note {
        eprintln!("gp: note: {note}");
    }
    let summary = format!(
        "traced frames {start} to {} of {} ({}): {} instructions, {} interrupts",
        end - 1,
        opened.entry.name,
        &opened.entry.id.to_hex()[..12],
        traced.instructions,
        traced.interrupts
    );
    let trace_text = String::from_utf8(buffer).expect("the trace is ASCII");
    let text = match out {
        Some(path) => format!("{summary}\nwritten to {}", path.display()),
        None => trace_text.trim_end().to_owned(),
    };
    Ok(Outcome {
        json: json!({
            "id": opened.entry.id,
            "frames": [start, end],
            "instructions": traced.instructions,
            "interrupts": traced.interrupts,
            "out": out,
            "trace": out.is_none().then_some(trace_text),
            "matches_recording": note.is_none(),
        }),
        text,
        exit: u8::from(note.is_some()),
    })
}

/// The store's SQLite index, brought up to date with the recordings log.
fn warn_index_problems(index: &Index) -> Result<()> {
    for (line, message) in index.bad_lines()? {
        eprintln!("gp: skipping recordings index line {line}, which can't be read: {message}");
    }
    Ok(())
}

fn recordings_list(filter: crate::recindex::Filter, paths: &Paths) -> Result<Outcome> {
    let store = Store::open(&paths.store)?;
    let index = Index::read(&store)?;
    recordings_list_in(filter, &index)
}

/// `gp recordings list` over an index that's already up to date. `gp serve`
/// passes one built in memory, so serving never writes the store.
fn recordings_list_in(filter: crate::recindex::Filter, index: &Index) -> Result<Outcome> {
    warn_index_problems(index)?;
    let unreadable = index.unreadable()?;
    if filter.reads_manifests() && !unreadable.is_empty() {
        eprintln!(
            "gp: {} recordings are left out because their manifests can't be read; `gp recordings show` says why",
            unreadable.len()
        );
    }
    let filtered = filter.reads_manifests()
        || filter.golden
        || filter.retired == crate::recindex::Retired::Only;
    let entries = index.list(&filter)?;
    let retired = index.retired()?;
    let hidden = if filter.retired == crate::recindex::Retired::Skip {
        index
            .list(&crate::recindex::Filter {
                retired: crate::recindex::Retired::Only,
                ..filter.clone()
            })?
            .len()
    } else {
        0
    };
    let mut text = if entries.is_empty() && filtered {
        "No recordings match.".to_owned()
    } else if entries.is_empty() && hidden > 0 {
        "Every recording is retired.".to_owned()
    } else if entries.is_empty() {
        "No recordings yet. `gp record` makes one (docs/recording.md).".to_owned()
    } else {
        entries
            .iter()
            .map(|e| {
                format!(
                    "{}  {:<20} {:>7} frames  {:<16} {}{}",
                    &e.id.to_hex()[..12],
                    e.source.as_deref().unwrap_or("-"),
                    e.frames,
                    &e.created_at[..19.min(e.created_at.len())],
                    e.name,
                    if retired.contains(&e.id) {
                        "  (retired)"
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    if hidden > 0 {
        text.push_str(&format!(
            "\n{hidden} retired entries not shown; --retired includes them"
        ));
    }
    let json_entries: Vec<Value> = entries
        .iter()
        .map(|e| {
            let mut v = json!(e);
            v["retired"] = json!(retired.contains(&e.id));
            v
        })
        .collect();
    Ok(Outcome::ok(
        json!({ "recordings": json_entries, "retired_hidden": hidden }),
        text,
    ))
}

fn recordings_show(id: &str, paths: &Paths) -> Result<Outcome> {
    let store = Store::open(&paths.store)?;
    let index = Index::read(&store)?;
    recordings_show_in(id, &store, &index)
}

/// `gp recordings show` over an index that's already up to date, as
/// [`recordings_list_in`].
fn recordings_show_in(id: &str, store: &Store, index: &Index) -> Result<Outcome> {
    let entry = index.resolve(id)?;
    let names = index.entries_for(&entry.id)?;
    let marks = index.marks_for(&entry.id)?;
    let retired = index.retired()?.contains(&entry.id);
    let Some(row) = index.recording(&entry.id)? else {
        let why = index
            .unreadable()?
            .into_iter()
            .find(|u| u.id == entry.id)
            .map_or_else(|| "it isn't in the index".to_owned(), |u| u.error);
        bail!(
            "recording {} ({}) can't be shown: {why}",
            entry.id,
            entry.name
        );
    };
    // Sizes as the store has them now, so a deleted object shows as missing.
    let objects: Vec<_> = index
        .objects(&entry.id)?
        .into_iter()
        .map(|mut o| {
            o.bytes = fs::metadata(store.path_of(&o.hash)).ok().map(|m| m.len());
            o
        })
        .collect();
    // A model recording is checked against the interpreter, a ROM's
    // against gp-nes.
    let current = gp_nes::runtime_ref();
    // Another build with the same behaviour version, whose fork patch
    // differs: its checkpoints restore if they round-trip, which replay
    // checks, so it isn't called stale here.
    let patch_only =
        gp_nes::recording::lineage_of(&row.runtime) == gp_nes::recording::Lineage::PatchOnly;
    let stale = row.runtime != current && !patch_only;

    let mut kinds: Vec<(&str, u64, u64)> = Vec::new();
    for o in &objects {
        match kinds.iter_mut().find(|k| k.0 == o.kind) {
            Some(k) => {
                k.1 += 1;
                k.2 += o.bytes.unwrap_or(0);
            }
            None => kinds.push((&o.kind, 1, o.bytes.unwrap_or(0))),
        }
    }
    let total: u64 = objects.iter().filter_map(|o| o.bytes).sum();
    let missing: Vec<_> = objects.iter().filter(|o| o.bytes.is_none()).collect();

    let mut lines = vec![format!("id          {}", entry.id)];
    for e in &names {
        lines.push(format!(
            "name        {}  ({}{}{})",
            e.name,
            &e.created_at[..19.min(e.created_at.len())],
            e.notes
                .as_deref()
                .map(|n| format!(", notes: {n}"))
                .unwrap_or_default(),
            if e.golden { ", golden" } else { "" }
        ));
    }
    for m in &marks {
        lines.push(format!(
            "{:<11} {}{}",
            match m.mark {
                gp_evidence::recordings::MarkKind::Retired => "retired",
                gp_evidence::recordings::MarkKind::Unretired => "unretired",
                gp_evidence::recordings::MarkKind::Deleted => "deleted",
            },
            &m.at[..19.min(m.at.len())],
            m.note
                .as_deref()
                .map(|n| format!(", notes: {n}"))
                .unwrap_or_default()
        ));
    }
    lines.push(if row.source_kind == "model" {
        format!(
            "source      the model {}, bundle {}",
            row.source_id.as_deref().unwrap_or("?"),
            row.source_sha256
        )
    } else {
        format!(
            "source      {}, {} mapper {}, sha256 {}",
            row.source_id.as_deref().unwrap_or("unregistered"),
            row.source_kind,
            row.mapper.map_or_else(|| "?".into(), |m| m.to_string()),
            row.source_sha256
        )
    });
    lines.push(format!(
        "runtime     {}{}",
        gp_nes::recording::describe_runtime(&row.runtime),
        if patch_only {
            format!(
                "  other patch: this is {}, with the same behaviour version; `gp replay` checks that its checkpoints still round-trip",
                gp_nes::recording::describe_runtime(&current)
            )
        } else if stale {
            format!(
                "  STALE: this is {}",
                gp_nes::recording::describe_runtime(&current)
            )
        } else {
            String::new()
        }
    ));
    lines.push(format!(
        "frames      {}, a checkpoint every {} frames",
        row.frames, row.checkpoint_interval
    ));
    lines.push(format!(
        "provenance  {}",
        row.provenance["kind"].as_str().unwrap_or("?")
    ));
    lines.push(format!("power-on    {}", row.power_on));
    lines.push(format!(
        "manifest    schema {}, frame hashes {}, trace {}, events {}",
        row.schema,
        row.frame_hash_format
            .map_or_else(|| "none".into(), |v| format!("format {v}")),
        if row.has_trace { "yes" } else { "none" },
        if row.has_events { "yes" } else { "none" },
    ));
    lines.push("objects".into());
    for (kind, count, bytes) in &kinds {
        lines.push(format!("  {kind:<14} {count:>5}  {bytes:>10} bytes"));
    }
    lines.push(format!(
        "  {:<14} {:>5}  {:>10} bytes",
        "total",
        objects.len(),
        total
    ));
    for o in &missing {
        lines.push(format!("  MISSING {} {}", o.kind, o.hash));
    }
    Ok(Outcome::ok(
        json!({
            "id": entry.id,
            "names": names,
            "retired": retired,
            "marks": marks,
            "recording": row,
            "stale": stale,
            "patch_only": patch_only,
            "objects": objects,
            "bytes": total,
            "missing": missing.len(),
        }),
        lines.join("\n"),
    ))
}

fn warn_bad_index_lines(listing: &gp_evidence::recordings::Listing) {
    for (line, message) in &listing.bad_lines {
        eprintln!("gp: skipping recordings index line {line}, which can't be read: {message}");
    }
}

fn replay_all(paths: &Paths, include_retired: bool) -> Result<Outcome> {
    let store = Store::open_existing(&paths.store)?;
    let listing = gp_evidence::recordings::list_checked(&store)?;
    warn_bad_index_lines(&listing);
    let retired = gp_evidence::recordings::retired(&listing.marks);
    let (entries, left_out): (Vec<_>, Vec<_>) = gp_evidence::recordings::unique(listing.entries)
        .into_iter()
        .partition(|e| include_retired || !retired.contains(&e.id));
    // With nothing to replay, no ROM folder is needed.
    let mut roms = RomFolder::new(paths);
    if !entries.is_empty() {
        roms.scan()?;
    }
    let (mut lines, mut results) = (Vec::new(), Vec::new());
    let (mut matched, mut diverged, mut skipped, mut failed) = (0u64, 0u64, 0u64, 0u64);
    let (mut stale, mut other) = (0u64, 0u64);
    for entry in &entries {
        let short = &entry.id.to_hex()[..12];
        // One recording that can't be checked doesn't stop the others.
        let checked = (|| -> Result<Option<Result<gp_nes::recording::Checked, ModelRun>>> {
            let manifest = gp_evidence::recordings::load_manifest(&store, &entry.id)?;
            let Some(sha256) = manifest.source.nes_sha256() else {
                return Ok(Some(Err(ModelRun)));
            };
            let Some(rom) = roms.read(sha256)? else {
                return Ok(None);
            };
            Ok(Some(Ok(gp_nes::recording::check(&store, &manifest, &rom)?)))
        })();
        match checked {
            Ok(None) => {
                skipped += 1;
                lines.push(format!(
                    "skip     {short}  {}  (no ROM with its hash in {})",
                    entry.name,
                    paths.roms.display()
                ));
            }
            Ok(Some(Err(ModelRun))) => {
                // A run of a model in the interpreter: the old tool replayed
                // it there; gameprinter has no model recordings yet.
                skipped += 1;
                lines.push(format!(
                    "skip     {short}  {}  (a model recording, which gameprinter doesn't replay yet)",
                    entry.name
                ));
            }
            Ok(Some(Ok(c))) => {
                use gp_nes::recording::Checked;
                lines.push(c.line(short, &entry.name));
                let mut result = match &c {
                    Checked::Replayed {
                        frames,
                        divergence,
                        recorded_by,
                    } => {
                        json!({ "frames": frames, "frame": divergence.as_ref().map(|d| d.frame), "runtime": recorded_by })
                    }
                    Checked::Stale {
                        frames,
                        divergence,
                        runtime,
                    } => {
                        json!({ "frames": frames, "frame": divergence.as_ref().map(|d| d.frame), "runtime": runtime })
                    }
                    Checked::StaleFormat { runtime, version } => {
                        json!({ "frames": 0, "frame_hash_format": version, "runtime": runtime })
                    }
                    Checked::OtherRuntime {
                        runtime,
                        divergence,
                    } => {
                        json!({ "frames": 0, "frame": divergence.as_ref().map(|d| d.frame), "runtime": runtime })
                    }
                };
                match c.result() {
                    "ok" => matched += 1,
                    "diverged" => diverged += 1,
                    "other" => other += 1,
                    _ => stale += 1,
                }
                result["id"] = json!(entry.id);
                result["result"] = json!(c.result());
                results.push(result);
            }
            Err(e) => {
                failed += 1;
                lines.push(format!("ERROR    {short}  {}  {e:#}", entry.name));
                results
                    .push(json!({ "id": entry.id, "result": "error", "error": format!("{e:#}") }));
            }
        }
    }
    if entries.is_empty() && !left_out.is_empty() {
        lines.push(
            "Every recording in the store is retired, so nothing was checked; --retired replays them."
                .into(),
        );
    } else if entries.is_empty() {
        lines.push("No recordings in the store, so nothing was checked.".into());
    } else {
        lines.push(format!(
            "{matched} recordings replayed identically, {diverged} diverged, {stale} stale, {failed} couldn't be checked, {skipped} skipped"
        ));
    }
    if other > 0 {
        lines.push(format!(
            "{other} recordings were made by another emulator (such as Mesen, by gp oracle record), so they weren't replayed; their traces were checked"
        ));
    }
    if !left_out.is_empty() {
        lines.push(format!(
            "{} retired recordings not replayed; --retired includes them",
            left_out.len()
        ));
    }
    if stale > 0 {
        lines.push(format!(
            "{stale} recordings were made by another runtime, and this is {}. Stale recordings don't fail the check, but their checkpoints can't be checked: re-record them (docs/recording.md).",
            gp_nes::recording::describe_runtime(&gp_nes::runtime_ref())
        ));
    }
    // Recordings exist but none had its ROM: almost always the wrong ROM
    // folder, and a check that checked nothing mustn't pass.
    // So does one where every recording is retired.
    let all_retired = entries.is_empty() && !left_out.is_empty();
    let nothing_checked = all_retired || (!entries.is_empty() && skipped == entries.len() as u64);
    if nothing_checked && !all_retired {
        lines.push(format!(
            "Every recording was skipped, so nothing was checked. Is {} the right ROM folder?",
            paths.roms.display()
        ));
    }
    let exit = if failed > 0 {
        2
    } else {
        u8::from(diverged > 0 || nothing_checked)
    };
    Ok(Outcome {
        json: json!({ "results": results, "matched": matched, "diverged": diverged, "stale": stale, "other": other, "failed": failed, "skipped": skipped, "retired": left_out.len() }),
        text: lines.join("\n"),
        exit,
    })
}

fn field_selected(name: &str, filters: &[String]) -> bool {
    filters.is_empty()
        || filters.iter().any(|f| {
            name == f
                || name
                    .strip_prefix(f.as_str())
                    .is_some_and(|rest| rest.starts_with('.'))
        })
}

/// A field's value as text: an integer for up to 4 bytes (little-endian),
/// otherwise a hex dump, 16 bytes a line.
fn field_text(bytes: &[u8], summarize: bool) -> String {
    match bytes.len() {
        0 => "(none)".into(),
        1..=4 => {
            let v = bytes
                .iter()
                .rev()
                .fold(0u64, |acc, b| (acc << 8) | u64::from(*b));
            format!("{:#0w$x} ({v})", v, w = 2 + 2 * bytes.len())
        }
        n if summarize && n > 32 => format!(
            "{n} bytes, BLAKE3 {}… (--field to show)",
            &Hash::of(bytes).to_hex()[..12]
        ),
        _ => bytes
            .chunks(16)
            .enumerate()
            .map(|(i, line)| {
                let hex: Vec<String> = line.iter().map(|b| format!("{b:02x}")).collect();
                format!("\n  {:04x}: {}", i * 16, hex.join(" "))
            })
            .collect(),
    }
}

fn trace_frame(
    id: &str,
    frame: u64,
    filters: &[String],
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let opened = Opened::open(paths, id)?;
    trace_frame_of(&opened, frame, filters, rom, paths)
}

/// `gp trace frame` for a recording that's already open.
fn trace_frame_of(
    opened: &Opened,
    frame: u64,
    filters: &[String],
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let (state, from) = opened.state(frame, rom, paths)?;
    let table = gp_nes::recording::field_table(&opened.store, &opened.manifest)?;
    let fields: Vec<_> = table
        .split(&state)?
        .into_iter()
        .filter(|(name, _)| field_selected(name, filters))
        .collect();
    if fields.is_empty() {
        bail!(
            "no field matches {}; the fields are {}",
            filters.join(","),
            table
                .fields
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let width = fields.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    let mut text = format!("frame {frame} of {}, from the {from}", opened.short());
    for (name, bytes) in &fields {
        text.push_str(&format!(
            "\n{name:<width$}  {}",
            field_text(bytes, filters.is_empty())
        ));
    }
    let json_fields: serde_json::Map<String, Value> = fields
        .iter()
        .map(|(n, b)| {
            (
                (*n).to_owned(),
                json!(b.iter().map(|x| format!("{x:02x}")).collect::<String>()),
            )
        })
        .collect();
    Ok(Outcome::ok(
        json!({ "id": opened.entry.id, "frame": frame, "from": from, "fields": json_fields }),
        text,
    ))
}

/// The bytes of two values of one field that differ, as `(offset, a, b)`.
fn differing_bytes(a: &[u8], b: &[u8]) -> Vec<(usize, Option<u8>, Option<u8>)> {
    (0..a.len().max(b.len()))
        .filter_map(|i| {
            let (x, y) = (a.get(i).copied(), b.get(i).copied());
            (x != y).then_some((i, x, y))
        })
        .collect()
}

fn trace_diff(a: &str, b: &str, paths: &Paths) -> Result<Outcome> {
    let a = Opened::new(paths, a)?;
    let b = Opened::new(paths, b)?;
    trace_diff_of(&a, &b, paths)
}

/// `gp trace diff` for two recordings that are already open.
fn trace_diff_of(a: &Opened, b: &Opened, paths: &Paths) -> Result<Outcome> {
    let ha = gp_nes::recording::frame_hashes_of(&a.store, &a.manifest)?;
    let hb = gp_nes::recording::frame_hashes_of(&b.store, &b.manifest)?;
    let common = ha.len().min(hb.len());
    let names = format!("a: {}\nb: {}", a.short(), b.short());
    let Some(frame) = (0..common).find(|&f| ha[f].state != hb[f].state) else {
        let (fa, fb) = (a.manifest.frames, b.manifest.frames);
        let (text, exit) = if fa == fb {
            (
                format!("{names}\nidentical: every frame's state matches, frames 0 to {fa}"),
                0,
            )
        } else {
            (
                format!(
                    "{names}\nframes 0 to {} match, but a has {fa} frames and b has {fb}",
                    common - 1
                ),
                1,
            )
        };
        return Ok(Outcome {
            json: json!({ "a": a.entry.id, "b": b.entry.id, "frame": null, "frames": [fa, fb] }),
            text,
            exit,
        });
    };
    let frame = frame as u64;
    let what = if ha[frame as usize].ram != hb[frame as usize].ram {
        "internal RAM differs"
    } else {
        "RAM matches, other state differs"
    };
    let mut text = format!("{names}\nfirst difference at frame {frame}: {what}");
    let (sa, from_a) = a.state(frame, None, paths)?;
    let (sb, from_b) = b.state(frame, None, paths)?;
    let ta = gp_nes::recording::field_table(&a.store, &a.manifest)?;
    let tb = gp_nes::recording::field_table(&b.store, &b.manifest)?;
    let mut fields = Vec::new();
    if ta != tb {
        text.push_str(
            "\nthe two states follow different field tables, so fields can't be compared",
        );
    } else {
        for ((name, x), (_, y)) in ta.split(&sa)?.into_iter().zip(tb.split(&sb)?) {
            let bytes = differing_bytes(x, y);
            if bytes.is_empty() {
                continue;
            }
            let line = if x.len() <= 4 && x.len() == y.len() {
                format!(
                    "{name}: {} -> {}",
                    field_text(x, true).split(' ').next().unwrap_or_default(),
                    field_text(y, true).split(' ').next().unwrap_or_default()
                )
            } else {
                let shown: Vec<String> = bytes
                    .iter()
                    .take(8)
                    .map(|(i, x, y)| {
                        let v = |b: &Option<u8>| b.map_or("--".into(), |b| format!("{b:02x}"));
                        format!("+{i:#06x} {}->{}", v(x), v(y))
                    })
                    .collect();
                let more = bytes.len().saturating_sub(8);
                format!(
                    "{name}: {} bytes differ: {}{}",
                    bytes.len(),
                    shown.join(", "),
                    if more > 0 {
                        format!(", {more} more")
                    } else {
                        String::new()
                    }
                )
            };
            text.push_str(&format!("\n  {line}"));
            fields.push(json!({
                "field": name,
                "bytes": bytes.iter().map(|(i, x, y)| json!([i, x, y])).collect::<Vec<_>>(),
            }));
        }
    }
    Ok(Outcome {
        json: json!({
            "a": a.entry.id,
            "b": b.entry.id,
            "frame": frame,
            "ram_differs": ha[frame as usize].ram != hb[frame as usize].ram,
            "from": [from_a, from_b],
            "fields": fields,
        }),
        text,
        exit: 1,
    })
}

/// `gp trace fields`: the field table, from the trace or, for a recording
/// without one, this runtime's.
fn trace_fields(opened: &Opened) -> Result<Outcome> {
    let table = gp_nes::recording::field_table(&opened.store, &opened.manifest)?;
    let magic = String::from_utf8_lossy(&table.state_magic).into_owned();
    let fields: Vec<Value> = table
        .fields
        .iter()
        .map(|f| match f.width {
            gp_evidence::trace::Width::Fixed(n) => {
                json!({ "name": f.name, "kind": "fixed", "width": n })
            }
            gp_evidence::trace::Width::Region => {
                json!({ "name": f.name, "kind": "region", "width": null })
            }
        })
        .collect();
    let mut text = format!(
        "{}: state {magic} version {}, {} fields",
        opened.short(),
        table.state_version,
        fields.len()
    );
    for f in &table.fields {
        text.push_str(&format!(
            "\n{:<28} {}",
            f.name,
            match f.width {
                gp_evidence::trace::Width::Fixed(n) => format!("{n} bytes"),
                gp_evidence::trace::Width::Region => "region".into(),
            }
        ));
    }
    Ok(Outcome::ok(
        json!({
            "id": opened.entry.id,
            "from": if opened.manifest.trace.is_some() { "trace" } else { "runtime" },
            "state_magic": magic,
            "state_version": table.state_version,
            "fields": fields,
        }),
        text,
    ))
}

fn trace_verify(id: &str, paths: &Paths) -> Result<Outcome> {
    let opened = Opened::open(paths, id)?;
    let start = Instant::now();
    let found = gp_nes::recording::verify_trace(&opened.store, &opened.manifest)?;
    let seconds = start.elapsed().as_secs_f64();
    let frames = opened.manifest.frames + 1;
    let (text, exit) = match &found {
        None => (
            format!(
                "{}: the trace matches the frame hashes, frames 0 to {} ({:.0} frames/s)",
                opened.short(),
                opened.manifest.frames,
                frames as f64 / seconds.max(1e-9)
            ),
            0,
        ),
        Some(d) => (
            format!(
                "{}: the trace doesn't match the frame hashes at frame {}",
                opened.short(),
                d.frame
            ),
            1,
        ),
    };
    Ok(Outcome {
        json: json!({ "id": opened.entry.id, "frame": found.map(|d| d.frame) }),
        text,
        exit,
    })
}

fn events(
    id: &str,
    frames: &str,
    kinds: &[String],
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let opened = Opened::open(paths, id)?;
    events_of(&opened, frames, kinds, rom, paths)
}

/// `gp events` for a recording that's already open.
fn events_of(
    opened: &Opened,
    frames: &str,
    kinds: &[String],
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    use gp_evidence::events::EventKind;
    let range = parse_frames(frames, opened.manifest.frames)?;
    refuse_model(opened)?;
    let kinds = kinds
        .iter()
        .map(|k| {
            EventKind::from_name(k).with_context(|| {
                format!(
                    "unknown event kind {k:?}; the kinds are {}",
                    EventKind::ALL.map(EventKind::name).join(", ")
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let (all, from) =
        match gp_nes::recording::stored_events(&opened.store, &opened.manifest, range.clone())? {
            Some(events) => (events, "event stream"),
            None => {
                let rom = opened.rom(rom, paths)?.bytes;
                (
                    gp_nes::recording::replayed_events(
                        &opened.store,
                        &opened.manifest,
                        &rom,
                        range.clone(),
                    )?,
                    "replay",
                )
            }
        };
    let shown: Vec<_> = all
        .into_iter()
        .filter(|e| kinds.is_empty() || kinds.contains(&e.kind))
        .collect();
    let mut text = format!(
        "{} events in frames {} to {} of {}, from the {from}",
        shown.len(),
        range.start,
        range.end.saturating_sub(1),
        opened.short()
    );
    for e in &shown {
        let detail = match e.kind {
            EventKind::Nmi | EventKind::Irq => format!("handler ${:04x}", e.addr),
            EventKind::Sprite0Hit => String::new(),
            EventKind::ControllerRead => format!("${:04x} -> ${:02x}", e.addr, e.value),
            _ => format!("${:04x} = ${:02x}", e.addr, e.value),
        };
        text.push_str(&format!(
            "\nframe {:>6}  {:>3}:{:<3}  {:<16} {detail}",
            e.frame,
            e.scanline,
            e.dot,
            e.kind.name()
        ));
    }
    let json_events: Vec<Value> = shown
        .iter()
        .map(|e| {
            json!({
                "frame": e.frame,
                "scanline": e.scanline,
                "dot": e.dot,
                "kind": e.kind.name(),
                "addr": e.addr,
                "value": e.value,
            })
        })
        .collect();
    Ok(Outcome::ok(
        json!({ "id": opened.entry.id, "frames": [range.start, range.end], "from": from, "events": json_events }),
        text.trim_end().to_owned(),
    ))
}

/// `gp movie`, with `gp record` for its `--record`.
pub fn movie_cmd(cmd: &crate::movie::MovieCmd, paths: &Paths) -> Result<Outcome> {
    if let crate::movie::MovieCmd::Diagnose { id } = cmd {
        let store = Store::open(&paths.store)?;
        let entry = gp_evidence::recordings::resolve(&store, id)?;
        let manifest = gp_evidence::recordings::load_manifest(&store, &entry.id)?;
        let text = String::from_utf8(store.get(&manifest.input_log)?)
            .context("the recording's input log isn't text")?;
        let log = gp_evidence::InputLog::parse(&text)?;
        let d = crate::diagnose::diagnose(&log);
        let mut json = serde_json::to_value(&d)?;
        json["id"] = serde_json::Value::String(entry.id.to_string());
        return Ok(Outcome::ok(json, crate::diagnose::text(&d)));
    }
    let registry = crate::open::load_registry()?;
    let mut record_log = |source: Option<&str>,
                          rom: Option<&Path>,
                          input: &Path,
                          label: IndexLabel<'_>,
                          provenance: gp_evidence::manifest::Provenance,
                          ram_init: &str| {
        record(
            source,
            rom,
            input,
            label,
            Keep {
                checkpoint_interval: 60,
                capture: gp_nes::recording::Capture {
                    trace: Some(gp_evidence::chunk::DEFAULT_CHUNK_FRAMES),
                    events: Some(gp_evidence::chunk::DEFAULT_CHUNK_FRAMES),
                },
                power_on: power_on(ram_init, "ntsc"),
            },
            provenance,
            paths,
        )
    };
    crate::movie::run(cmd, &registry, &mut record_log)
}
