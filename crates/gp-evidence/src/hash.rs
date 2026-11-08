use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A BLAKE3 content hash. Everything in the evidence store is addressed by one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hash([u8; 32]);

impl Hash {
    pub fn of(data: &[u8]) -> Self {
        Self(*blake3::hash(data).as_bytes())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn from_blake3(h: blake3::Hash) -> Self {
        Self(*h.as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.to_hex())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("not a hash: expected 64 hex characters, got {0:?}")]
pub struct ParseHashError(String);

impl FromStr for Hash {
    type Err = ParseHashError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut out = [0u8; 32];
        hex::decode_to_slice(s, &mut out).map_err(|_| ParseHashError(s.to_owned()))?;
        Ok(Self(out))
    }
}

/// A hash is 64 lowercase hex digits in JSON and TOML.
impl schemars::JsonSchema for Hash {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Hash".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[0-9a-f]{64}$",
            "description": "A BLAKE3 hash, as 64 lowercase hex digits"
        })
    }
}

impl Serialize for Hash {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let h = Hash::of(b"game printer");
        let parsed: Hash = h.to_hex().parse().unwrap();
        assert_eq!(h, parsed);
    }

    #[test]
    fn rejects_bad_hex() {
        assert!("abc".parse::<Hash>().is_err());
        assert!("zz".repeat(32).parse::<Hash>().is_err());
    }

    #[test]
    fn serializes_as_hex_string() {
        let h = Hash::of(b"x");
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, format!("\"{h}\""));
        assert_eq!(serde_json::from_str::<Hash>(&json).unwrap(), h);
    }
}
