//! Validation results kept in the store, as coverage keeps its own
//! (gp-coverage's `cache`): each recording's [`RecordingResult`] is a store
//! object, its JSON, and one line in `<store>/index/validation.jsonl`
//! points at it by key:
//!
//! ```text
//! {"key":"<hash>","validate_version":1,"recording":"<id>","object":"<hash>","at":"2026-10-28T00:00:00Z"}
//! ```
//!
//! The key is the hash of everything the result depends on: this version,
//! the model's and contract's canonical text, the state mapping, the
//! recording, and the runtimes that ran it ([`key`]). Results can always be
//! evaluated again, so a line whose object is missing or unreadable is
//! passed over.

use std::path::PathBuf;

use gp_evidence::{Hash, Store};
use serde::{Deserialize, Serialize};

use crate::{RecordingResult, VALIDATE_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("validation index {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Store(#[from] gp_evidence::store::StoreError),
}

/// One line of the validation index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub key: Hash,
    pub validate_version: u32,
    /// The recording's ID, or a baseline's path.
    pub recording: String,
    pub object: Hash,
    /// RFC 3339.
    pub at: String,
}

/// What a result depends on, hashed.
#[derive(Debug, Clone, Serialize)]
pub struct KeyParts<'a> {
    pub model: &'a str,
    pub contract: &'a str,
    pub mapping: &'a str,
    /// The recording's ID, or what identifies a baseline run: its ROM and
    /// input.
    pub recording: &'a str,
    pub original: &'a str,
    pub subject: Option<&'a str>,
}

pub fn key(parts: &KeyParts<'_>) -> Hash {
    let text = serde_json::json!({
        "validate_version": VALIDATE_VERSION,
        "parts": parts,
    });
    Hash::of(text.to_string().as_bytes())
}

/// `<store>/index/validation.jsonl`.
pub fn log_path(store: &Store) -> PathBuf {
    store.root().join("index").join("validation.jsonl")
}

pub use gp_evidence::jsonl::BadLines;

/// Every readable line, and the lines that can't be read. `gp store gc`
/// keeps every object a line names and refuses to run while one can't be
/// read.
pub fn lines_checked(store: &Store) -> Result<(Vec<Line>, BadLines), CacheError> {
    let path = log_path(store);
    gp_evidence::jsonl::read_checked(&path).map_err(|source| CacheError::Io { path, source })
}

/// The stored result with this key, if there is one that reads back.
pub fn lookup(store: &Store, key: Hash) -> Result<Option<RecordingResult>, CacheError> {
    let (lines, _) = lines_checked(store)?;
    for line in lines
        .into_iter()
        .rev()
        .filter(|l| l.key == key && l.validate_version == VALIDATE_VERSION)
    {
        let Ok(bytes) = store.get(&line.object) else {
            continue;
        };
        if let Some(r) = serde_json::from_slice::<RecordingResult>(&bytes)
            .ok()
            .filter(|r| r.key == key.to_hex() && r.validate_version == VALIDATE_VERSION)
        {
            return Ok(Some(r));
        }
    }
    Ok(None)
}

/// Stores one recording's result and adds its line to the index.
pub fn save(store: &Store, result: &RecordingResult, at: &str) -> Result<Hash, CacheError> {
    let bytes = serde_json::to_vec(result).expect("results serialize");
    let object = store.put_bytes(&bytes)?;
    let line = Line {
        key: result.key.parse().expect("a result's key is a hash"),
        validate_version: result.validate_version,
        recording: result.recording.clone(),
        object,
        at: at.to_owned(),
    };
    let mut text = serde_json::to_string(&line).expect("lines serialize");
    text.push('\n');
    let path = log_path(store);
    gp_evidence::jsonl::append(&path, &text).map_err(|source| CacheError::Io { path, source })?;
    Ok(object)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_result_is_found_by_its_key_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let parts = KeyParts {
            model: "m",
            contract: "c",
            mapping: "map",
            recording: "r",
            original: "gp-nes 6",
            subject: Some("gp-nes 6"),
        };
        let k = key(&parts);
        assert_ne!(
            k,
            key(&KeyParts {
                subject: None,
                ..parts.clone()
            })
        );
        assert_eq!(lookup(&store, k).unwrap(), None);
        let result = RecordingResult {
            validate_version: VALIDATE_VERSION,
            key: k.to_hex(),
            recording: "r".into(),
            name: "run".into(),
            subject: Some("gp-nes 6".into()),
            frames: 3,
            outcomes: Vec::new(),
        };
        let object = save(&store, &result, "2026-10-28T00:00:00Z").unwrap();
        assert_eq!(lookup(&store, k).unwrap(), Some(result));
        let (lines, bad) = lines_checked(&store).unwrap();
        assert_eq!((lines.len(), lines[0].object, bad.len()), (1, object, 0));
        // A damaged line is reported, and the good ones still read.
        let mut log = std::fs::read_to_string(log_path(&store)).unwrap();
        log.push_str("{not json\n");
        std::fs::write(log_path(&store), log).unwrap();
        let (lines, bad) = lines_checked(&store).unwrap();
        assert_eq!((lines.len(), bad.len()), (1, 1));
    }
}
