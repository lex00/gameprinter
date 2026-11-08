//! A game's text encoding: which tile number stands for each character.
//! In the map it's a list of runs, each a first tile number and the
//! characters from it on:
//!
//! ```toml
//! charset = [[0x00, "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ "], [0x28, "-"]]
//! ```
//!
//! Formats that hold tile numbers write a run of them as a string when
//! every one has a character, and as a list of numbers otherwise, so any
//! bytes round-trip.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct Charset(Vec<(u8, String)>);

impl Charset {
    /// Checks every character is printable ASCII (a space included) other
    /// than `"` and `\`, and that no character or tile is given twice.
    pub fn validate(&self, what: &str) -> Result<(), String> {
        let mut chars = std::collections::BTreeSet::new();
        let mut tiles = std::collections::BTreeSet::new();
        for (first, run) in &self.0 {
            for (i, c) in run.chars().enumerate() {
                if !(c == ' ' || c.is_ascii_graphic()) || c == '"' || c == '\\' {
                    return Err(format!(
                        "{what}: charset character {c:?} must be printable ASCII other than \" or \\"
                    ));
                }
                let tile = u32::from(*first) + i as u32;
                if tile > 0xFF {
                    return Err(format!(
                        "{what}: the charset run from {first:#04x} goes past tile 0xFF"
                    ));
                }
                if !chars.insert(c) || !tiles.insert(tile) {
                    return Err(format!(
                        "{what}: the charset gives character {c:?} or tile {tile:#04x} twice"
                    ));
                }
            }
        }
        Ok(())
    }

    fn char_of(&self, tile: u8) -> Option<char> {
        self.0.iter().find_map(|(first, run)| {
            tile.checked_sub(*first)
                .and_then(|i| run.chars().nth(i as usize))
        })
    }

    /// The tile number that stands for a character.
    pub fn tile(&self, c: char) -> Option<u8> {
        self.tile_of(c)
    }

    fn tile_of(&self, c: char) -> Option<u8> {
        self.0
            .iter()
            .find_map(|(first, run)| run.chars().position(|r| r == c).map(|i| first + i as u8))
    }

    /// The bytes as text, if there are any and every one has a character.
    pub fn decode(&self, bytes: &[u8]) -> Option<String> {
        if bytes.is_empty() {
            return None;
        }
        bytes.iter().map(|&b| self.char_of(b)).collect()
    }

    /// Text back into tile numbers.
    pub fn encode(&self, text: &str) -> Result<Vec<u8>, String> {
        text.chars()
            .map(|c| {
                self.tile_of(c)
                    .ok_or_else(|| format!("{c:?} isn't in the map's charset"))
            })
            .collect()
    }
}

/// A TOML string for text that [`Charset::validate`] allowed: no quote or
/// backslash to escape.
pub fn quoted(text: &str) -> String {
    format!("\"{text}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn charset() -> Charset {
        Charset(vec![(0x00, "0123456789AB ".into()), (0x28, "-!".into())])
    }

    #[test]
    fn text_round_trips_and_unknown_tiles_stay_numbers() {
        let c = charset();
        c.validate("t").unwrap();
        assert_eq!(c.decode(&[0x0A, 0x0B, 0x0C, 0x01]).unwrap(), "AB 1");
        assert_eq!(
            c.encode("AB 1-!").unwrap(),
            [0x0A, 0x0B, 0x0C, 0x01, 0x28, 0x29]
        );
        assert_eq!(c.decode(&[0x0A, 0x27]), None);
        assert_eq!(c.decode(&[]), None);
        assert!(c.encode("Z").unwrap_err().contains("'Z'"));
    }

    #[test]
    fn bad_charsets_are_refused() {
        let twice = Charset(vec![(0, "AB".into()), (5, "A".into())]);
        assert!(twice.validate("t").unwrap_err().contains("twice"));
        let tile_twice = Charset(vec![(0, "AB".into()), (1, "C".into())]);
        assert!(tile_twice.validate("t").unwrap_err().contains("twice"));
        let quote = Charset(vec![(0, "\"".into())]);
        assert!(quote.validate("t").is_err());
        let past = Charset(vec![(0xFF, "AB".into())]);
        assert!(past.validate("t").unwrap_err().contains("past"));
    }
}
