//! A fixed-size set of byte offsets, stored as ranges.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Which of `len` bytes are in the set. Serialized as the length and the
/// set's runs, `[start, end)` each, so a mostly empty or mostly full set
/// stays small and reads well as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bits {
    len: usize,
    words: Vec<u64>,
}

impl Bits {
    pub fn new(len: usize) -> Self {
        Self {
            len,
            words: vec![0; len.div_ceil(64)],
        }
    }

    /// How many bytes the set ranges over.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Adds `i`; an offset past the end is ignored.
    #[inline]
    pub fn set(&mut self, i: usize) {
        if i < self.len {
            self.words[i / 64] |= 1 << (i % 64);
        }
    }

    pub fn contains(&self, i: usize) -> bool {
        i < self.len && self.words[i / 64] & (1 << (i % 64)) != 0
    }

    /// The set's 64-bit words, bit `i % 64` of word `i / 64` for byte `i`.
    pub fn words(&self) -> &[u64] {
        &self.words
    }

    pub fn words_mut(&mut self) -> &mut [u64] {
        &mut self.words
    }

    /// Bytes in the set.
    pub fn count(&self) -> u64 {
        self.words.iter().map(|w| u64::from(w.count_ones())).sum()
    }

    /// Adds every byte of `other`. Both must range over the same length.
    pub fn union(&mut self, other: &Bits) -> Result<(), String> {
        if self.len != other.len {
            return Err(format!(
                "can't combine a set over {} bytes with one over {}",
                self.len, other.len
            ));
        }
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            *a |= b;
        }
        Ok(())
    }

    /// The set's runs, `[start, end)`, in order.
    pub fn ranges(&self) -> Vec<[usize; 2]> {
        let mut out = Vec::new();
        let mut start = None;
        for i in 0..self.len {
            match (self.contains(i), start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    out.push([s, i]);
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            out.push([s, self.len]);
        }
        out
    }

    fn from_ranges(len: usize, ranges: &[[usize; 2]]) -> Result<Self, String> {
        let mut bits = Self::new(len);
        for &[start, end] in ranges {
            if start >= end || end > len {
                return Err(format!("range {start}..{end} isn't within 0..{len}"));
            }
            for i in start..end {
                bits.set(i);
            }
        }
        Ok(bits)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    len: usize,
    ranges: Vec<[usize; 2]>,
}

impl Serialize for Bits {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Stored {
            len: self.len,
            ranges: self.ranges(),
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Bits {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let stored = Stored::deserialize(d)?;
        Bits::from_ranges(stored.len, &stored.ranges).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_runs_and_round_trips() {
        let mut bits = Bits::new(130);
        for i in [0, 1, 2, 63, 64, 65, 129, 500] {
            bits.set(i);
        }
        assert_eq!(bits.count(), 7);
        assert_eq!(bits.ranges(), vec![[0, 3], [63, 66], [129, 130]]);
        let json = serde_json::to_string(&bits).unwrap();
        assert_eq!(json, r#"{"len":130,"ranges":[[0,3],[63,66],[129,130]]}"#);
        assert_eq!(serde_json::from_str::<Bits>(&json).unwrap(), bits);
        assert!(serde_json::from_str::<Bits>(r#"{"len":4,"ranges":[[2,9]]}"#).is_err());

        let mut other = Bits::new(130);
        other.set(3);
        bits.union(&other).unwrap();
        assert_eq!(bits.ranges()[0], [0, 4]);
        assert!(bits.union(&Bits::new(8)).is_err());
    }
}
