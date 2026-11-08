//! Per-frame state hashes: what replay compares against.
//!
//! Binary, little-endian. A header (`GPFH`, format version u32, entry count
//! u64), then one entry per frame from 0 (power-on) to the last frame: the
//! BLAKE3 hash of the console's state, then the hash of its internal RAM, 32
//! bytes each. See docs/formats/recording.md.
//!
//! The layout is the same in every version; what changes is what the state
//! hash covers and where a frame ends. Version 0 hashed `tetanes-core`'s save
//! state at the start of scanline 240. Version 1 hashes the runtime's
//! canonical state (for NES, `gp_nes::canonical`) at the start of vertical
//! blank. Both decode; only a runtime that writes a version can check it.

use crate::hash::Hash;

pub const MAGIC: &[u8; 4] = b"GPFH";
/// The version [`encode`] writes.
pub const FORMAT_VERSION: u32 = 1;
/// Versions [`decode`] reads.
pub const READABLE_VERSIONS: [u32; 2] = [0, 1];
const HEADER: usize = 16;
const ENTRY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHash {
    /// The console's state: in version 1, the runtime's canonical state;
    /// in version 0, `tetanes-core`'s save state.
    pub state: Hash,
    /// Internal RAM only, where game logic lives. Kept separately so a
    /// difference can be told apart from one in emulator-internal state.
    pub ram: Hash,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameHashesError {
    #[error("not a frame hash file (missing GPFH header)")]
    Magic,
    #[error("frame hash format {0} is not supported")]
    Version(u32),
    #[error("frame hash file is truncated: header says {expected} entries, data holds {actual}")]
    Length { expected: u64, actual: u64 },
}

pub fn encode(hashes: &[FrameHash]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + hashes.len() * ENTRY);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&(hashes.len() as u64).to_le_bytes());
    for h in hashes {
        out.extend_from_slice(h.state.as_bytes());
        out.extend_from_slice(h.ram.as_bytes());
    }
    out
}

/// The format version in the header, which says what the hashes mean.
pub fn version(bytes: &[u8]) -> Result<u32, FrameHashesError> {
    if bytes.len() < HEADER || &bytes[..4] != MAGIC {
        return Err(FrameHashesError::Magic);
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
    if !READABLE_VERSIONS.contains(&version) {
        return Err(FrameHashesError::Version(version));
    }
    Ok(version)
}

/// The entries of any readable version. Check [`version`] before comparing
/// them with hashes a runtime computes now.
pub fn decode(bytes: &[u8]) -> Result<Vec<FrameHash>, FrameHashesError> {
    version(bytes)?;
    let expected = u64::from_le_bytes(bytes[8..16].try_into().expect("8 bytes"));
    let body = &bytes[HEADER..];
    let actual = (body.len() / ENTRY) as u64;
    if !body.len().is_multiple_of(ENTRY) || actual != expected {
        return Err(FrameHashesError::Length { expected, actual });
    }
    Ok(body
        .as_chunks::<ENTRY>()
        .0
        .iter()
        .map(|e| FrameHash {
            state: Hash::from_bytes(e[..32].try_into().expect("32 bytes")),
            ram: Hash::from_bytes(e[32..].try_into().expect("32 bytes")),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let hashes: Vec<FrameHash> = (0u8..5)
            .map(|i| FrameHash {
                state: Hash::of(&[i]),
                ram: Hash::of(&[i, i]),
            })
            .collect();
        let bytes = encode(&hashes);
        assert_eq!(bytes.len(), 16 + 5 * 64);
        assert_eq!(decode(&bytes).unwrap(), hashes);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(decode(b"nope"), Err(FrameHashesError::Magic));
        let mut bytes = encode(&[FrameHash {
            state: Hash::of(b"a"),
            ram: Hash::of(b"b"),
        }]);
        bytes.truncate(bytes.len() - 1);
        assert!(matches!(
            decode(&bytes),
            Err(FrameHashesError::Length { .. })
        ));
        let mut v = encode(&[]);
        v[4] = 9;
        assert_eq!(decode(&v), Err(FrameHashesError::Version(9)));
    }

    #[test]
    fn reads_version_0() {
        let hashes = [FrameHash {
            state: Hash::of(b"save state"),
            ram: Hash::of(b"ram"),
        }];
        let mut bytes = encode(&hashes);
        assert_eq!(version(&bytes), Ok(FORMAT_VERSION));
        bytes[4..8].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(version(&bytes), Ok(0));
        assert_eq!(decode(&bytes).unwrap(), hashes);
    }
}
