//! The save record: one game kept by a player, as the generated game's
//! save slots and the web preview's both write it (docs/settings.md,
//! "Saves"). It holds the canonical state (`World::canonical`) with what
//! the state leaves out: the game it is of, the build that made it (the
//! spec's digest in the save's profile) and the profile.
//!
//! A build loads only its own saves. The canonical state's version changes
//! with its layout, not with the model, so a save of an older model can
//! read as a state of the new one and be wrong; the digest names the build
//! exactly, and a save of any other build is refused with [`OTHER_BUILD`].
//!
//! Layout, all numbers little-endian: `GPSV`, the version (u32), then the
//! model's name, the digest and the profile, each a u16 length and UTF-8,
//! then the canonical state to the end. Version 1 (the generated game's
//! until 2026-11-04) had no digest and is refused.

use crate::Spec;

/// A save record's first bytes.
pub const MAGIC: &[u8; 4] = b"GPSV";
/// The record's version.
pub const VERSION: u32 = 2;

/// What a player is told when a save was made by another build of the
/// game, or by a version of the game from before saves named their build.
/// Each sentence fits the generated game's message line on a 256-pixel
/// screen (41 letters).
pub const OTHER_BUILD: &str = "Can't load a save from another version";
/// What a player is told when a save is of another game.
pub const OTHER_GAME: &str = "Can't load a save of another game";
/// What a player is told when the bytes aren't a save, or are cut short.
pub const NOT_A_SAVE: &str = "Can't load this save: it is damaged";

/// BLAKE3 of a spec's debug form, in hex: the digest a generated game
/// carries (`headless --spec-digest`) and the preview reports
/// (`gp_spec_digest`).
pub fn spec_digest(spec: &Spec) -> String {
    blake3::hash(format!("{spec:?}").as_bytes())
        .to_hex()
        .to_string()
}

/// One save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The record's version as read: [`VERSION`] for a record this build
    /// can load.
    pub version: u32,
    /// The model's name (`Spec::name`).
    pub model: String,
    /// The spec digest of the build that made it, in the save's profile.
    /// Empty in a version 1 record.
    pub digest: String,
    /// The profile it was played in, which the state leaves out.
    pub profile: String,
    /// The canonical state.
    pub state: Vec<u8>,
}

impl Record {
    /// A record of this version.
    pub fn new(model: &str, digest: &str, profile: &str, state: Vec<u8>) -> Self {
        Record {
            version: VERSION,
            model: model.to_owned(),
            digest: digest.to_owned(),
            profile: profile.to_owned(),
            state,
        }
    }

    /// The record's bytes, always in this [`VERSION`].
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.model.len() + 64 + self.state.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        for s in [&self.model, &self.digest, &self.profile] {
            out.extend_from_slice(&(s.len() as u16).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        out.extend_from_slice(&self.state);
        out
    }

    /// Reads a record of version 1 or 2 without judging it, so a slot list
    /// can still show an old save's level and time. [`Record::check`] says
    /// whether it loads. Errors are [`NOT_A_SAVE`] and [`OTHER_BUILD`]
    /// (a version this build doesn't know).
    pub fn read(bytes: &[u8]) -> Result<Record, String> {
        if bytes.get(..4) != Some(MAGIC) {
            return Err(NOT_A_SAVE.into());
        }
        let mut at = 4;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let b = bytes.get(at..at + n).ok_or(NOT_A_SAVE)?;
            at += n;
            Ok(b)
        };
        let version = u32::from_le_bytes(take(4)?.try_into().map_err(|_| NOT_A_SAVE)?);
        if version != 1 && version != VERSION {
            return Err(OTHER_BUILD.into());
        }
        let mut text = || -> Result<String, String> {
            let len = u16::from_le_bytes(take(2)?.try_into().map_err(|_| NOT_A_SAVE)?) as usize;
            String::from_utf8(take(len)?.to_vec()).map_err(|_| NOT_A_SAVE.to_owned())
        };
        let model = text()?;
        let digest = if version == 1 { String::new() } else { text()? };
        let profile = text()?;
        Ok(Record {
            version,
            model,
            digest,
            profile,
            state: bytes[at..].to_vec(),
        })
    }

    /// The record if this build of `model` can load it: the same game, this
    /// version, and the digest `digest_of` gives for the save's profile.
    /// A profile the build doesn't know is another build's too.
    pub fn check(
        self,
        model: &str,
        digest_of: impl FnOnce(&str) -> Result<String, String>,
    ) -> Result<Record, String> {
        if self.model != model {
            return Err(OTHER_GAME.into());
        }
        if self.version != VERSION {
            return Err(OTHER_BUILD.into());
        }
        match digest_of(&self.profile) {
            Ok(d) if d == self.digest => Ok(self),
            _ => Err(OTHER_BUILD.into()),
        }
    }

    /// [`Record::read`] then [`Record::check`].
    pub fn load(
        bytes: &[u8],
        model: &str,
        digest_of: impl FnOnce(&str) -> Result<String, String>,
    ) -> Result<Record, String> {
        Record::read(bytes)?.check(model, digest_of)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(p: &str) -> Result<String, String> {
        match p {
            "retro" => Ok("d1".into()),
            "normal" => Ok("d2".into()),
            _ => Err(format!("no profile {p}")),
        }
    }

    #[test]
    fn a_record_reads_back_and_loads_only_in_its_own_build() {
        let r = Record::new("mosshop", "d1", "retro", vec![1, 2, 3, 4]);
        let bytes = r.to_bytes();
        assert_eq!(&bytes[..8], b"GPSV\x02\x00\x00\x00");
        assert_eq!(Record::read(&bytes).unwrap(), r);
        assert_eq!(Record::load(&bytes, "mosshop", digest).unwrap(), r);
        // Another build, another game, a profile the build lacks.
        let other = Record::new("mosshop", "d0", "retro", vec![1]).to_bytes();
        assert_eq!(
            Record::load(&other, "mosshop", digest).unwrap_err(),
            OTHER_BUILD
        );
        assert_eq!(
            Record::load(&bytes, "other", digest).unwrap_err(),
            OTHER_GAME
        );
        let lost = Record::new("mosshop", "d1", "gone", vec![1]).to_bytes();
        assert_eq!(
            Record::load(&lost, "mosshop", digest).unwrap_err(),
            OTHER_BUILD
        );
        // Not a save, or cut short.
        assert_eq!(Record::read(b"GPSM").unwrap_err(), NOT_A_SAVE);
        assert_eq!(Record::read(&bytes[..12]).unwrap_err(), NOT_A_SAVE);
        // A version this build doesn't know.
        let mut v3 = bytes.clone();
        v3[4] = 3;
        assert_eq!(Record::read(&v3).unwrap_err(), OTHER_BUILD);
        for m in [OTHER_BUILD, OTHER_GAME, NOT_A_SAVE] {
            assert!(m.len() <= 41, "{m}");
        }
    }

    #[test]
    fn a_version_1_save_reads_for_its_slot_line_and_is_refused() {
        // Version 1: GPSV, 1, the model and the profile, then the state.
        let mut v1 = b"GPSV\x01\x00\x00\x00".to_vec();
        for s in ["mosshop", "retro"] {
            v1.extend_from_slice(&(s.len() as u16).to_le_bytes());
            v1.extend_from_slice(s.as_bytes());
        }
        v1.extend_from_slice(&[9, 9]);
        let r = Record::read(&v1).unwrap();
        assert_eq!(
            (r.version, r.profile.as_str(), r.state.as_slice()),
            (1, "retro", &[9, 9][..])
        );
        assert_eq!(r.check("mosshop", digest).unwrap_err(), OTHER_BUILD);
    }
}
