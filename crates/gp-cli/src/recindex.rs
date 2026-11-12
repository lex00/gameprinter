//! The recordings index `gp recordings list` and `show` read, built from
//! the store's recordings log and the manifests it names each time.
//!
//! The old tool kept this in a SQLite file beside the store (gp-index),
//! brought up to date with the log before each read. gameprinter scans the
//! log instead: it gives the same answers, lists and rows as gp-index did,
//! with no index file to keep, no C build of SQLite and nothing written to
//! the store. A store with many thousands of recordings would want the
//! SQLite index back; this one has a few.

use std::collections::HashSet;
use std::fs;
use std::io::Read as _;
use std::path::Path;

use anyhow::Result;
use gp_evidence::frame_hashes;
use gp_evidence::manifest::{RecordingManifest, RuntimeRef, SourceRef};
use gp_evidence::recordings::{self, IndexEntry, Mark};
use gp_evidence::{Hash, Store};
use serde::Serialize;

/// Which entries [`Index::list`] returns. Every field narrows the list.
#[derive(Debug, Clone, Default)]
pub(crate) struct Filter {
    /// A registry ID, or a prefix of the source's SHA-256.
    pub source: Option<String>,
    /// Text the runtime's name and version (`gp-nes 2 (...)`) contain.
    pub runtime: Option<String>,
    /// Only recordings made by a runtime other than this one.
    pub stale_against: Option<RuntimeRef>,
    /// Only golden recordings.
    pub golden: bool,
    /// Whether retired recordings are left out (the default), included, or
    /// the only ones listed.
    pub retired: Retired,
}

/// How [`Filter`] treats retired recordings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Retired {
    #[default]
    Skip,
    Include,
    Only,
}

impl Filter {
    /// Whether the filter needs a recording's manifest, so leaves out
    /// recordings whose manifest can't be read.
    pub fn reads_manifests(&self) -> bool {
        self.source.is_some() || self.runtime.is_some() || self.stale_against.is_some()
    }
}

/// A recording's manifest, as gp-index's row held it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RecordingRow {
    pub id: Hash,
    /// The schema the manifest was written in, before any conversion.
    pub schema: u32,
    pub source_kind: String,
    pub source_sha256: String,
    pub mapper: Option<u16>,
    /// The registry ID from the latest log entry that has one.
    pub source_id: Option<String>,
    pub runtime: RuntimeRef,
    pub power_on: serde_json::Value,
    pub frames: u64,
    pub checkpoint_interval: u32,
    pub provenance: serde_json::Value,
    /// The frame hash object's format version, if it has one and its header
    /// could be read.
    pub frame_hash_format: Option<u32>,
    pub has_trace: bool,
    pub has_events: bool,
}

/// One store object a recording uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ObjectRef {
    /// `manifest`, `input_log`, `frame_hashes`, `checkpoint`,
    /// `trace_fields` (the frame trace's field table), `trace` (its chunks)
    /// or `events`, as the manifest names them.
    pub kind: String,
    /// Position among the recording's objects of this kind.
    pub ord: u64,
    /// For a checkpoint, its frame.
    pub frame: Option<u64>,
    pub hash: Hash,
    /// Its size now, or none if it isn't in the store.
    pub bytes: Option<u64>,
}

/// A recording in the log whose manifest is missing or can't be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Unreadable {
    pub id: Hash,
    pub error: String,
}

struct Read {
    row: RecordingRow,
    objects: Vec<ObjectRef>,
}

/// The log, its marks and every manifest it names, read once.
pub(crate) struct Index {
    entries: Vec<IndexEntry>,
    marks: Vec<Mark>,
    bad_lines: Vec<(usize, String)>,
    read: Vec<(Hash, std::result::Result<Read, String>)>,
}

impl Index {
    pub fn read(store: &Store) -> Result<Self> {
        let listing = recordings::list_checked(store)?;
        let mut read: Vec<(Hash, std::result::Result<Read, String>)> = Vec::new();
        let mut ids: Vec<Hash> = listing.entries.iter().map(|e| e.id).collect();
        ids.sort_by_key(|h| h.to_hex());
        ids.dedup();
        for id in ids {
            // The registry ID from the latest entry that has one.
            let source_id = listing
                .entries
                .iter()
                .rev()
                .find(|e| e.id == id && e.source.is_some())
                .and_then(|e| e.source.clone());
            read.push((id, read_recording(store, &id, source_id)));
        }
        Ok(Self {
            entries: listing.entries,
            marks: listing.marks,
            bad_lines: listing.bad_lines,
            read,
        })
    }

    fn row(&self, id: &Hash) -> Option<&RecordingRow> {
        self.read
            .iter()
            .find(|(h, _)| h == id)
            .and_then(|(_, r)| r.as_ref().ok())
            .map(|r| &r.row)
    }

    /// Log entries, oldest first, narrowed by `filter`. An ID recorded under
    /// several names appears once per name.
    pub fn list(&self, filter: &Filter) -> Result<Vec<IndexEntry>> {
        let retired = self.retired()?;
        let golden: HashSet<Hash> = self
            .entries
            .iter()
            .filter(|e| e.golden)
            .map(|e| e.id)
            .collect();
        Ok(self
            .entries
            .iter()
            .filter(|e| {
                let row = self.row(&e.id);
                let source_ok = filter.source.as_ref().is_none_or(|s| {
                    e.source.as_ref() == Some(s)
                        || row.is_some_and(|r| {
                            r.source_id.as_ref() == Some(s)
                                || r.source_sha256.starts_with(&s.to_ascii_lowercase())
                        })
                });
                let runtime_ok = filter.runtime.as_ref().is_none_or(|t| {
                    row.is_some_and(|r| {
                        format!("{} {}", r.runtime.name, r.runtime.version).contains(t.as_str())
                    })
                });
                let stale_ok = filter.stale_against.as_ref().is_none_or(|now| {
                    // An arcade recording is stale against gp-arcade.
                    row.is_some_and(|r| {
                        if r.source_kind == "romset" {
                            r.runtime != crate::arcade_rec::runtime_ref()
                        } else {
                            &r.runtime != now
                        }
                    })
                });
                let golden_ok = !filter.golden || golden.contains(&e.id);
                let retired_ok = match filter.retired {
                    Retired::Include => true,
                    Retired::Only => retired.contains(&e.id),
                    Retired::Skip => !retired.contains(&e.id),
                };
                source_ok && runtime_ok && stale_ok && golden_ok && retired_ok
            })
            .cloned()
            .collect())
    }

    /// Finds a recording by a prefix of its ID (at least 4 hex characters),
    /// as [`recordings::resolve`] does on the log.
    pub fn resolve(&self, prefix: &str) -> Result<IndexEntry> {
        let prefix = prefix.to_ascii_lowercase();
        if prefix.len() < 4 {
            return Err(recordings::RecordingsError::TooShort(prefix).into());
        }
        let found: Vec<IndexEntry> = self
            .entries
            .iter()
            .filter(|e| e.id.to_hex().starts_with(&prefix))
            .cloned()
            .collect();
        let mut matches = recordings::unique(found);
        match matches.len() {
            0 => Err(recordings::RecordingsError::NotFound(prefix).into()),
            1 => Ok(matches.remove(0)),
            count => Err(recordings::RecordingsError::Ambiguous { prefix, count }.into()),
        }
    }

    /// The IDs of every retired recording: those whose latest mark retires
    /// them.
    pub fn retired(&self) -> Result<HashSet<Hash>> {
        Ok(recordings::retired(&self.marks))
    }

    /// Every mark for one ID, oldest first.
    pub fn marks_for(&self, id: &Hash) -> Result<Vec<Mark>> {
        Ok(self.marks.iter().filter(|m| &m.id == id).cloned().collect())
    }

    /// Every log entry for one ID, oldest first.
    pub fn entries_for(&self, id: &Hash) -> Result<Vec<IndexEntry>> {
        Ok(self
            .entries
            .iter()
            .filter(|e| &e.id == id)
            .cloned()
            .collect())
    }

    /// A recording's manifest summary, if its manifest could be read.
    pub fn recording(&self, id: &Hash) -> Result<Option<RecordingRow>> {
        Ok(self.row(id).cloned())
    }

    /// The store objects a recording uses, its manifest first.
    pub fn objects(&self, id: &Hash) -> Result<Vec<ObjectRef>> {
        Ok(self
            .read
            .iter()
            .find(|(h, _)| h == id)
            .and_then(|(_, r)| r.as_ref().ok())
            .map(|r| r.objects.clone())
            .unwrap_or_default())
    }

    /// Recordings in the log whose manifest is missing or can't be read.
    pub fn unreadable(&self) -> Result<Vec<Unreadable>> {
        Ok(self
            .read
            .iter()
            .filter_map(|(id, r)| {
                r.as_ref().err().map(|error| Unreadable {
                    id: *id,
                    error: error.clone(),
                })
            })
            .collect())
    }

    /// Log lines that couldn't be read: line numbers (from 1) and errors.
    pub fn bad_lines(&self) -> Result<Vec<(usize, String)>> {
        Ok(self.bad_lines.clone())
    }
}

/// Reads a recording's manifest, or says why it can't.
fn read_recording(
    store: &Store,
    id: &Hash,
    source_id: Option<String>,
) -> std::result::Result<Read, String> {
    #[derive(serde::Deserialize)]
    struct Schema {
        schema: u32,
    }
    let bytes = store.get(id).map_err(|e| e.to_string())?;
    let schema = serde_json::from_slice::<Schema>(&bytes)
        .map_err(|e| format!("manifest JSON: {e}"))?
        .schema;
    let m = RecordingManifest::from_json(&bytes).map_err(|e| e.to_string())?;
    let size = |h: &Hash| fs::metadata(store.path_of(h)).ok().map(|m| m.len());
    let mut objects = Vec::new();
    let mut add = |kind: &str, ord: usize, frame: Option<u64>, hash: Hash| {
        objects.push(ObjectRef {
            kind: kind.to_owned(),
            ord: ord as u64,
            frame,
            hash,
            bytes: size(&hash),
        });
    };
    add("manifest", 0, None, *id);
    add("input_log", 0, None, m.input_log);
    if let SourceRef::Model { bundle, .. } = &m.source {
        add("model_bundle", 0, None, *bundle);
    }
    if let Some(h) = m.frame_hashes {
        add("frame_hashes", 0, None, h);
    }
    for (i, c) in m.checkpoints.iter().enumerate() {
        add("checkpoint", i, Some(c.frame), c.snapshot);
    }
    if let Some(t) = &m.trace {
        add("trace_fields", 0, None, t.fields);
        for (i, h) in t.chunks.iter().enumerate() {
            add("trace", i, None, *h);
        }
    }
    if let Some(e) = &m.events {
        for (i, h) in e.chunks.iter().enumerate() {
            add("events", i, None, *h);
        }
    }
    let frame_hash_format = m
        .frame_hashes
        .and_then(|h| frame_hash_format(&store.path_of(&h)));
    let (source_sha256, mapper) = match &m.source {
        SourceRef::NesRom { sha256, mapper } => (sha256.clone(), Some(*mapper)),
        SourceRef::Model { bundle, .. } => (bundle.to_hex(), None),
        SourceRef::Romset { identity, .. } => (identity.clone(), None),
    };
    let row = RecordingRow {
        id: *id,
        schema,
        source_kind: m.source.kind().to_owned(),
        source_sha256,
        mapper,
        source_id,
        runtime: m.runtime.clone(),
        power_on: serde_json::to_value(m.power_on).map_err(|e| e.to_string())?,
        frames: m.frames,
        checkpoint_interval: m.checkpoint_interval,
        provenance: serde_json::to_value(&m.provenance).map_err(|e| e.to_string())?,
        frame_hash_format,
        has_trace: m.trace.is_some(),
        has_events: m.events.is_some(),
    };
    Ok(Read { row, objects })
}

/// The format version in a frame hash object's header. Only the header is
/// read; replay checks the rest.
fn frame_hash_format(path: &Path) -> Option<u32> {
    let mut header = [0u8; 16];
    fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    frame_hashes::version(&header).ok()
}
