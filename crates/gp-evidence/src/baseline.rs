//! Baselines: scripted runs whose results are committed, so a check can
//! record them again and compare. A baseline file holds the input as button
//! spans, the expected frame hash digest and RAM assertions at fixed frames.
//! None of it comes from the ROM. They live in the repo under `baselines/`;
//! docs/recording.md describes the format.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::hash::Hash;
use crate::input_log::{self, InputLog};

/// The baseline file format version.
pub const BASELINE_SCHEMA: u32 = 0;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub schema: u32,
    /// The registry ID of the source, for messages.
    #[serde(default)]
    pub source: Option<String>,
    /// Headerless SHA-256 of the ROM, as in the registry; the ROM is found by it.
    pub sha256: String,
    /// What the run does, in a sentence.
    #[serde(default)]
    pub description: Option<String>,
    /// Frames to run.
    pub frames: usize,
    /// Button spans for controller 1, as `gp input-log generate --hold` takes them.
    pub holds: Vec<String>,
    /// Expected hash of the recording's frame hash object, as hex.
    pub frame_hashes: String,
    /// Expected number of lag frames.
    pub lag_frames: usize,
    #[serde(default, rename = "assert")]
    pub asserts: Vec<RamAssert>,
}

/// A RAM byte's expected value at a frame.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RamAssert {
    pub frame: u64,
    /// The disassembly's name for the address.
    pub label: String,
    /// Address in the console's 2KB of internal RAM.
    pub addr: u16,
    pub equals: u8,
}

#[derive(Debug, thiserror::Error)]
pub enum BaselineError {
    // The source error's message follows in the error chain.
    #[error("{path}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{path}: {message}")]
    Invalid { path: PathBuf, message: String },
}

impl Baseline {
    pub fn load(path: &Path) -> Result<Self, BaselineError> {
        let text = std::fs::read_to_string(path).map_err(|source| BaselineError::Io {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(&text, path)
    }

    /// Parses and validates a baseline's text; `path` is only for messages.
    /// Builds that can't read files (gp-wasm-check under wasmtime) embed the
    /// text instead.
    pub fn parse(text: &str, path: &Path) -> Result<Self, BaselineError> {
        let baseline: Self = toml::from_str(text).map_err(|source| BaselineError::Parse {
            path: path.to_owned(),
            source,
        })?;
        baseline
            .validate()
            .map_err(|message| BaselineError::Invalid {
                path: path.to_owned(),
                message,
            })?;
        Ok(baseline)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != BASELINE_SCHEMA {
            return Err(format!(
                "baseline schema {} is not supported (this gp reads {BASELINE_SCHEMA})",
                self.schema
            ));
        }
        if self.frames == 0 {
            return Err("frames must be at least 1".into());
        }
        self.expected_frame_hashes()?;
        self.input().map_err(|e| e.to_string())?;
        for a in &self.asserts {
            if a.frame > self.frames as u64 {
                return Err(format!(
                    "assert {} at frame {} is past the last frame, {}",
                    a.label, a.frame, self.frames
                ));
            }
            if a.addr >= 0x800 {
                return Err(format!(
                    "assert {} reads ${:04X}, outside internal RAM ($0000-$07FF)",
                    a.label, a.addr
                ));
            }
        }
        Ok(())
    }

    /// The input log the spans describe.
    pub fn input(&self) -> Result<InputLog, input_log::InputLogError> {
        let spans: Vec<&str> = self.holds.iter().map(String::as_str).collect();
        input_log::generate(self.frames, &spans)
    }

    pub fn expected_frame_hashes(&self) -> Result<Hash, String> {
        let bytes: [u8; 32] = hex::decode(&self.frame_hashes)
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| {
                format!(
                    "frame_hashes must be 64 hex characters, got {:?}",
                    self.frame_hashes
                )
            })?;
        Ok(Hash::from_bytes(bytes))
    }
}

/// The nearest `baselines/` directory at or above `start`.
pub fn find_dir(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|d| d.join("baselines"))
        .find(|d| d.is_dir())
}

/// Every `.toml` file under `dir`, sorted.
pub fn list(dir: &Path) -> Result<Vec<PathBuf>, BaselineError> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), BaselineError> {
        let io = |source| BaselineError::Io {
            path: dir.to_owned(),
            source,
        };
        for entry in std::fs::read_dir(dir).map_err(io)? {
            let path = entry.map_err(io)?.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.extension().is_some_and(|e| e == "toml") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, &mut out)?;
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
schema = 0
source = "test"
sha256 = "00"
frames = 100
holds = ["start@10-12", "right@20-"]
frame_hashes = "0000000000000000000000000000000000000000000000000000000000000000"
lag_frames = 3

[[assert]]
frame = 100
label = "Counter"
addr = 0x10
equals = 7
"#;

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn loads_and_builds_the_input() {
        let tmp = tempfile::tempdir().unwrap();
        let b = Baseline::load(&write(tmp.path(), "a.toml", GOOD)).unwrap();
        assert_eq!(b.asserts.len(), 1);
        assert_eq!(b.asserts[0].addr, 0x10);
        let input = b.input().unwrap();
        assert_eq!(input.frames.len(), 100);
        assert_eq!(input.frames[11].pads[0], input_log::button::START);
        assert_eq!(input.frames[99].pads[0], input_log::button::RIGHT);
    }

    #[test]
    fn rejects_bad_baselines() {
        let tmp = tempfile::tempdir().unwrap();
        for (bad, why) in [
            (GOOD.replace("frame = 100", "frame = 101"), "past the last"),
            (
                GOOD.replace("addr = 0x10", "addr = 0x6000"),
                "outside internal RAM",
            ),
            (GOOD.replace("\"0000000000", "\"00"), "64 hex"),
            (GOOD.replace("right@20-", "right@20-200"), "isn't within"),
            (GOOD.replace("schema = 0", "schema = 9"), "schema 9"),
        ] {
            let err = Baseline::load(&write(tmp.path(), "b.toml", &bad)).unwrap_err();
            assert!(err.to_string().contains(why), "{err}");
        }
        let err = Baseline::load(&write(tmp.path(), "c.toml", &format!("{GOOD}\nextra = 1")))
            .unwrap_err();
        assert!(matches!(err, BaselineError::Parse { .. }), "{err}");
    }

    #[test]
    fn lists_toml_files_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("baselines/game")).unwrap();
        write(&tmp.path().join("baselines/game"), "run.toml", GOOD);
        write(&tmp.path().join("baselines"), "README.md", "");
        let dir = find_dir(&tmp.path().join("baselines/game")).unwrap();
        assert_eq!(dir, tmp.path().join("baselines"));
        let files = list(&dir).unwrap();
        assert_eq!(files, vec![tmp.path().join("baselines/game/run.toml")]);
    }
}
