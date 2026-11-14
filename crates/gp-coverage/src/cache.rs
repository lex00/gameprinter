//! Coverage results kept in the store, so a corpus report measures only the
//! recordings it hasn't measured before.
//!
//! Each recording's [`Coverage`] is a store object (its JSON), and one line
//! in `<store>/index/coverage.jsonl` points at it, keyed by recording,
//! [`COVERAGE_VERSION`] and the definition's hash:
//!
//! ```text
//! {"recording":"<id>","coverage_version":1,"definition":"<hash>","object":"<hash>","at":"2026-10-27T00:00:00Z"}
//! ```
//!
//! The objects can always be measured again from the recordings, so a line
//! whose object is missing or unreadable is passed over, not an error.

use std::path::PathBuf;

use gp_evidence::{Hash, Store};
use serde::{Deserialize, Serialize};

use crate::{COVERAGE_VERSION, Coverage};

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("coverage index {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Store(#[from] gp_evidence::store::StoreError),
}

/// One line of the coverage index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub recording: Hash,
    pub coverage_version: u32,
    /// The definition's hash, or `null` for coverage measured without one.
    pub definition: Option<Hash>,
    pub object: Hash,
    /// RFC 3339.
    pub at: String,
}

/// `<store>/index/coverage.jsonl`.
pub fn log_path(store: &Store) -> PathBuf {
    store.root().join("index").join("coverage.jsonl")
}

/// Every readable line of the index, in order.
pub fn lines(store: &Store) -> Result<Vec<Line>, CacheError> {
    lines_checked(store).map(|(good, _)| good)
}

pub use gp_evidence::jsonl::BadLines;

/// Every line of the index and the lines that can't be read, as line
/// numbers (from 1) and errors. `gp store gc` keeps every object a line
/// names, so it refuses to delete anything while a line can't be read.
pub fn lines_checked(store: &Store) -> Result<(Vec<Line>, BadLines), CacheError> {
    let path = log_path(store);
    gp_evidence::jsonl::read_checked(&path).map_err(|source| CacheError::Io { path, source })
}

/// The stored coverage of `recording` measured with `definition` by this
/// coverage version, if there is any.
pub fn lookup(
    store: &Store,
    recording: Hash,
    definition: Option<Hash>,
) -> Result<Option<Coverage>, CacheError> {
    // The newest line whose object is still there and reads back.
    for line in lines(store)?.into_iter().rev().filter(|l| {
        l.recording == recording
            && l.coverage_version == COVERAGE_VERSION
            && l.definition == definition
    }) {
        let Ok(bytes) = store.get(&line.object) else {
            continue;
        };
        if let Some(c) = serde_json::from_slice::<Coverage>(&bytes).ok().filter(|c| {
            c.recordings == [recording]
                && c.coverage_version == COVERAGE_VERSION
                && c.definition.as_ref().map(|d| d.hash) == definition
        }) {
            return Ok(Some(c));
        }
    }
    Ok(None)
}

/// Stores one recording's coverage and adds its line to the index.
pub fn save(store: &Store, coverage: &Coverage, at: &str) -> Result<Hash, CacheError> {
    let [recording] = coverage.recordings[..] else {
        panic!("only one recording's coverage is stored");
    };
    let bytes = serde_json::to_vec(coverage).expect("coverage serializes");
    let object = store.put_bytes(&bytes)?;
    let line = Line {
        recording,
        coverage_version: coverage.coverage_version,
        definition: coverage.definition.as_ref().map(|d| d.hash),
        object,
        at: at.to_owned(),
    };
    let mut text = serde_json::to_string(&line).expect("lines serialize");
    text.push('\n');
    let path = log_path(store);
    gp_evidence::jsonl::append(&path, &text).map_err(|source| CacheError::Io { path, source })?;
    Ok(object)
}
