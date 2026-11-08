//! The recordings index: one JSON line per recording in
//! `<store>/index/recordings.jsonl`, so recordings can be listed and found by
//! a short ID prefix. The manifests themselves are store objects; this file
//! only points at them. It's the durable record and reads on every platform,
//! WASM included. Native `gp` also keeps a SQLite index derived from it and
//! the manifests (the `gp-index` crate), which can always be rebuilt from
//! them.
//!
//! A recording's ID is the hash of what was run, so recording the same run
//! again gives the same ID. Each recording under a new name adds an entry, and
//! several entries can share an ID. Listing shows every entry; finding by ID
//! and replaying everything treat an ID once ([`unique`]).
//!
//! The log also holds marks ([`Mark`]): lines that set something about a
//! recording after the fact, such as retiring it. A mark is never removed;
//! a later mark for the same ID overrides it ([`retired`]). Nothing is
//! deleted and the manifest doesn't change, so a retired recording's
//! objects stay referenced and it still resolves by ID. A `gp` from before
//! marks existed reports a mark line as one it can't read and skips it.
//!
//! The one exception is a `deleted` mark, which `gp store gc --retired`
//! appends just before it deletes a retired recording's objects. Reading
//! the log then ignores every line for that ID up to and including its last
//! `deleted` mark, so the recording is gone from every listing and its
//! objects are unreferenced. Recording the same run again appends a new
//! entry after the mark, which brings the ID back with only its new lines.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hash::Hash;
use crate::manifest::RecordingManifest;
use crate::store::{Store, StoreError};

/// One named recording: what isn't part of the run itself, so isn't in the
/// manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    /// The manifest's hash, which is the recording's ID.
    pub id: Hash,
    /// RFC 3339.
    pub created_at: String,
    /// Source ID from the registry, such as `smb1-ntsc-prg0`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub frames: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Set once the recording's diff against the oracle has been triaged and
    /// accepted. It describes the run, so a recording is golden if any of its
    /// entries says so. `gp oracle golden` sets it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub golden: bool,
}

/// What a [`Mark`] sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkKind {
    /// `gp recordings list` and `gp replay --all` skip the recording unless
    /// asked for retired ones.
    Retired,
    /// Undoes an earlier `retired`.
    Unretired,
    /// `gp store gc --retired` deleted the recording's objects. Lines for
    /// the ID up to and including this one are ignored ([`list_checked`]).
    Deleted,
}

/// A log line that marks a recording, such as
/// `{"mark":"retired","id":"...","at":"2026-10-27T00:00:00Z"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mark {
    pub mark: MarkKind,
    pub id: Hash,
    /// RFC 3339.
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One line of the log.
#[derive(Deserialize)]
#[serde(untagged)]
enum Line {
    Mark(Mark),
    Entry(IndexEntry),
}

/// The IDs whose latest mark retires them.
pub fn retired(marks: &[Mark]) -> std::collections::HashSet<Hash> {
    let mut out = std::collections::HashSet::new();
    for m in marks {
        match m.mark {
            MarkKind::Retired => out.insert(m.id),
            MarkKind::Unretired | MarkKind::Deleted => out.remove(&m.id),
        };
    }
    out
}

/// Adds a mark to the log. Marks always append, so the log keeps the
/// history of retiring and unretiring.
pub fn append_mark(store: &Store, mark: &Mark) -> Result<(), RecordingsError> {
    let mut line = serde_json::to_string(mark).expect("marks serialize");
    line.push('\n');
    append_line(store, &line)
}

fn append_line(store: &Store, line: &str) -> Result<(), RecordingsError> {
    // `gp store gc` holds the store's lock while it reads the log and
    // deletes, so a line can't land halfway through.
    store.lock_for_writing()?;
    let path = log_path(store);
    let dir = path.parent().expect("index path has a parent");
    fs::create_dir_all(dir).map_err(io(dir))?;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(io(&path))?;
    // One write per line: appends from concurrent runs then can't
    // interleave inside a line.
    f.write_all(line.as_bytes()).map_err(io(&path))
}

#[derive(Debug, thiserror::Error)]
pub enum RecordingsError {
    #[error("recordings index {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("no recording matches {0:?}; `gp recordings list` shows them")]
    NotFound(String),
    #[error("{0:?} is too short; give at least 4 characters of the ID")]
    TooShort(String),
    #[error("{prefix:?} matches {count} recordings; use more characters")]
    Ambiguous { prefix: String, count: usize },
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("recording manifest {id}: {message}")]
    Manifest { id: Hash, message: String },
    /// The installed [`crate::guard`] refused the recording.
    #[error("{0}")]
    Refused(String),
}

/// `<store>/index/recordings.jsonl`.
pub fn log_path(store: &Store) -> PathBuf {
    store.root().join("index").join("recordings.jsonl")
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> RecordingsError + '_ {
    move |source| RecordingsError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Adds an entry, unless one with the same ID, name and golden status is
/// already there. Returns whether it added one.
pub fn append(store: &Store, entry: &IndexEntry) -> Result<bool, RecordingsError> {
    if list(store)?
        .iter()
        .any(|e| e.id == entry.id && e.name == entry.name && e.golden == entry.golden)
    {
        return Ok(false);
    }
    let mut line = serde_json::to_string(entry).expect("index entries serialize");
    line.push('\n');
    append_line(store, &line)?;
    Ok(true)
}

/// One entry per ID, in the order each ID first appears: the first creation
/// time, the latest entry's name, notes and source, and golden if any entry is.
pub fn unique(entries: Vec<IndexEntry>) -> Vec<IndexEntry> {
    let mut out: Vec<IndexEntry> = Vec::new();
    for e in entries {
        match out.iter_mut().find(|o| o.id == e.id) {
            Some(o) => {
                let golden = o.golden || e.golden;
                *o = IndexEntry {
                    golden,
                    created_at: std::mem::take(&mut o.created_at),
                    ..e
                };
            }
            None => out.push(e),
        }
    }
    out
}

/// The index, oldest first.
pub fn list(store: &Store) -> Result<Vec<IndexEntry>, RecordingsError> {
    Ok(list_checked(store)?.entries)
}

/// The index and any lines that couldn't be read.
#[derive(Debug, Default)]
pub struct Listing {
    pub entries: Vec<IndexEntry>,
    /// Marks, in log order.
    pub marks: Vec<Mark>,
    /// Line numbers (from 1) and parse errors of lines that were skipped.
    pub bad_lines: Vec<(usize, String)>,
    /// IDs whose objects `gp store gc` deleted, with no entry since: their
    /// lines are left out of `entries` and `marks`.
    pub deleted: Vec<Hash>,
}

/// Like [`list`], but also says which lines were skipped. A damaged line
/// doesn't hide the recordings on the other lines.
pub fn list_checked(store: &Store) -> Result<Listing, RecordingsError> {
    let path = log_path(store);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Listing::default()),
        Err(e) => return Err(io(&path)(e)),
    };
    let mut listing = Listing::default();
    // Each line read, with its position, so a `deleted` mark can drop the
    // lines before it.
    let mut entries = Vec::new();
    let mut marks = Vec::new();
    for (i, l) in text.lines().enumerate() {
        if l.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(l) {
            Ok(Line::Entry(e)) => entries.push((i, e)),
            Ok(Line::Mark(m)) => marks.push((i, m)),
            // The untagged enum's error says only that no variant matched,
            // so the entry's own error is the one to report.
            Err(_) => match serde_json::from_str::<IndexEntry>(l) {
                Ok(e) => entries.push((i, e)),
                Err(e) => listing.bad_lines.push((i + 1, e.to_string())),
            },
        }
    }
    let mut deleted_at: std::collections::HashMap<Hash, usize> = Default::default();
    for (i, m) in &marks {
        if m.mark == MarkKind::Deleted {
            deleted_at.insert(m.id, *i);
        }
    }
    let kept = |id: &Hash, i: usize| deleted_at.get(id).is_none_or(|d| i > *d);
    listing.entries = entries
        .into_iter()
        .filter(|(i, e)| kept(&e.id, *i))
        .map(|(_, e)| e)
        .collect();
    listing.marks = marks
        .into_iter()
        .filter(|(i, m)| kept(&m.id, *i))
        .map(|(_, m)| m)
        .collect();
    listing.deleted = deleted_at
        .keys()
        .filter(|id| !listing.entries.iter().any(|e| e.id == **id))
        .copied()
        .collect();
    listing.deleted.sort();
    Ok(listing)
}

/// Finds a recording by a prefix of its ID (at least 4 hex characters). An
/// ID with several entries is one match, merged as in [`unique`].
pub fn resolve(store: &Store, prefix: &str) -> Result<IndexEntry, RecordingsError> {
    let prefix = prefix.to_ascii_lowercase();
    if prefix.len() < 4 {
        return Err(RecordingsError::TooShort(prefix));
    }
    let mut matches = unique(
        list(store)?
            .into_iter()
            .filter(|e| e.id.to_hex().starts_with(&prefix))
            .collect(),
    );
    match matches.len() {
        0 => Err(RecordingsError::NotFound(prefix)),
        1 => Ok(matches.remove(0)),
        count => Err(RecordingsError::Ambiguous { prefix, count }),
    }
}

/// Reads and validates a recording's manifest from the store, converting
/// one written in an older schema ([`RecordingManifest::from_json`]). It is
/// the store's one way to open a recording, so the installed
/// [`crate::guard`] is asked here, and a recording it refuses is
/// [`RecordingsError::Refused`].
pub fn load_manifest(store: &Store, id: &Hash) -> Result<RecordingManifest, RecordingsError> {
    let m = load_manifest_unguarded(store, id)?;
    crate::guard::recording(store, id, &m).map_err(RecordingsError::Refused)?;
    Ok(m)
}

/// [`load_manifest`] without asking the guard. Only a reader that must see
/// what the guard refuses calls it: the guard itself, the coordinator's
/// scorer and its listing of the recordings the guard refuses.
pub fn load_manifest_unguarded(
    store: &Store,
    id: &Hash,
) -> Result<RecordingManifest, RecordingsError> {
    let bytes = store.get(id)?;
    RecordingManifest::from_json(&bytes).map_err(|e| RecordingsError::Manifest {
        id: *id,
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: u8) -> IndexEntry {
        IndexEntry {
            id: Hash::of(&[n]),
            created_at: "2026-10-27T00:00:00Z".into(),
            source: Some("test".into()),
            frames: 10,
            name: format!("r{n}"),
            notes: None,
            golden: false,
        }
    }

    #[test]
    fn append_list_resolve() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        assert!(list(&store).unwrap().is_empty());
        let (a, b) = (entry(1), entry(2));
        assert!(append(&store, &a).unwrap());
        assert!(append(&store, &b).unwrap());
        assert_eq!(list(&store).unwrap(), vec![a.clone(), b.clone()]);
        assert_eq!(resolve(&store, &a.id.to_hex()[..12]).unwrap(), a);
        assert!(matches!(
            resolve(&store, "ab"),
            Err(RecordingsError::TooShort(_))
        ));
        assert!(matches!(
            resolve(&store, "ffffffff"),
            Err(RecordingsError::NotFound(_))
        ));
    }

    #[test]
    fn entries_sharing_an_id_resolve_once() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let first = entry(1);
        let other = entry(2);
        let renamed = IndexEntry {
            name: "again".into(),
            created_at: "2026-10-28T00:00:00Z".into(),
            notes: Some("second take".into()),
            golden: false,
            ..first.clone()
        };
        let golden = IndexEntry {
            name: "golden".into(),
            golden: true,
            ..first.clone()
        };
        for e in [&first, &other, &renamed] {
            assert!(append(&store, e).unwrap());
        }
        // The same ID under the same name adds nothing.
        assert!(!append(&store, &first).unwrap());
        assert_eq!(list(&store).unwrap().len(), 3);

        let found = resolve(&store, &first.id.to_hex()[..8]).unwrap();
        assert_eq!(found.name, "again");
        assert_eq!(found.notes.as_deref(), Some("second take"));
        assert_eq!(found.created_at, first.created_at, "first recorded");
        append(&store, &golden).unwrap();
        let all = unique(list(&store).unwrap());
        assert_eq!(
            all.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![first.id, other.id]
        );
        assert!(all[0].golden && !all[1].golden);

        // Marking a recording golden under its own name adds an entry, once
        // (`gp oracle golden` keeps the name).
        let marked = IndexEntry {
            golden: true,
            ..other.clone()
        };
        assert!(append(&store, &marked).unwrap());
        assert!(!append(&store, &marked).unwrap());
    }

    #[test]
    fn marks_retire_and_unretire_without_hiding_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let (a, b) = (entry(1), entry(2));
        append(&store, &a).unwrap();
        append(&store, &b).unwrap();
        let mark = |mark, e: &IndexEntry| Mark {
            mark,
            id: e.id,
            at: "2026-10-28T00:00:00Z".into(),
            note: None,
        };
        append_mark(&store, &mark(MarkKind::Retired, &a)).unwrap();
        append_mark(&store, &mark(MarkKind::Retired, &b)).unwrap();
        append_mark(&store, &mark(MarkKind::Unretired, &b)).unwrap();
        let listing = list_checked(&store).unwrap();
        assert!(listing.bad_lines.is_empty(), "{:?}", listing.bad_lines);
        assert_eq!(listing.entries, vec![a.clone(), b.clone()]);
        assert_eq!(listing.marks.len(), 3);
        assert_eq!(
            retired(&listing.marks),
            std::collections::HashSet::from([a.id])
        );
        // A retired recording still resolves.
        assert_eq!(resolve(&store, &a.id.to_hex()[..8]).unwrap(), a);
        let text = fs::read_to_string(log_path(&store)).unwrap();
        assert!(text.contains(&format!("{{\"mark\":\"retired\",\"id\":\"{}\"", a.id)));
        // A damaged mark is a bad line, not an entry.
        let mut f = OpenOptions::new()
            .append(true)
            .open(log_path(&store))
            .unwrap();
        f.write_all(b"{\"mark\":\"lost\",\"id\":\"x\"}\n").unwrap();
        assert_eq!(list_checked(&store).unwrap().bad_lines.len(), 1);
    }

    #[test]
    fn a_deleted_mark_drops_the_lines_before_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let (a, b) = (entry(1), entry(2));
        append(&store, &a).unwrap();
        append(&store, &b).unwrap();
        let mark = |mark, e: &IndexEntry| Mark {
            mark,
            id: e.id,
            at: "2026-10-28T00:00:00Z".into(),
            note: None,
        };
        append_mark(&store, &mark(MarkKind::Retired, &a)).unwrap();
        append_mark(&store, &mark(MarkKind::Deleted, &a)).unwrap();
        let listing = list_checked(&store).unwrap();
        assert_eq!(listing.entries, vec![b.clone()]);
        assert!(listing.marks.is_empty());
        assert_eq!(listing.deleted, vec![a.id]);
        assert!(matches!(
            resolve(&store, &a.id.to_hex()[..8]),
            Err(RecordingsError::NotFound(_))
        ));
        // Recording the same run again brings it back, with only its new
        // lines: it isn't retired any more.
        assert!(append(&store, &a).unwrap());
        let listing = list_checked(&store).unwrap();
        assert_eq!(listing.entries, vec![b, a.clone()]);
        assert!(listing.deleted.is_empty());
        assert!(retired(&listing.marks).is_empty());
        assert_eq!(resolve(&store, &a.id.to_hex()[..8]).unwrap(), a);
    }

    #[test]
    fn schema_0_index_lines_still_read() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let path = log_path(&store);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let id = Hash::of(b"m");
        fs::write(
            &path,
            format!(
                "{{\"id\":\"{id}\",\"created_at\":\"2026-10-27T00:00:00Z\",\"source\":\"smb1-ntsc-prg0\",\"frames\":1800,\"name\":\"smb-run-right\"}}\n"
            ),
        )
        .unwrap();
        let e = resolve(&store, &id.to_hex()[..6]).unwrap();
        assert_eq!(
            (e.name.as_str(), e.notes, e.golden),
            ("smb-run-right", None, false)
        );
    }

    #[test]
    fn a_damaged_line_hides_only_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let (a, b) = (entry(1), entry(2));
        append(&store, &a).unwrap();
        let mut f = OpenOptions::new()
            .append(true)
            .open(log_path(&store))
            .unwrap();
        f.write_all(b"{\"id\": \"trunc\n").unwrap();
        append(&store, &b).unwrap();
        let listing = list_checked(&store).unwrap();
        assert_eq!(listing.entries, vec![a, b.clone()]);
        assert_eq!(listing.bad_lines.len(), 1);
        assert_eq!(listing.bad_lines[0].0, 2);
        assert_eq!(resolve(&store, &b.id.to_hex()[..8]).unwrap(), b);
    }
}
