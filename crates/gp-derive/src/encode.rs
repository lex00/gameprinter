//! The encodings a number in a game's code or data takes, and reading a
//! model value back out of one. NES games keep speeds in sixteenths or
//! 256ths of a pixel, signed bytes for directions, 8.8 words, and the odd
//! 4096ths for friction; a model holds all of them as 1/65536ths
//! ([`gp_model::fixed`]), so each encoding is a shift and a sign.

/// One way to read a raw number as a model value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Encoding {
    /// The number itself.
    Whole,
    /// A byte or word as two's complement.
    Signed,
    /// Sixteenths.
    Sixteenths,
    SignedSixteenths,
    /// 256ths.
    Fraction256,
    SignedFraction256,
    /// 4096ths.
    Fraction4096,
}

impl Encoding {
    pub fn name(self) -> &'static str {
        match self {
            Self::Whole => "whole",
            Self::Signed => "signed",
            Self::Sixteenths => "16ths",
            Self::SignedSixteenths => "signed 16ths",
            Self::Fraction256 => "256ths",
            Self::SignedFraction256 => "signed 256ths",
            Self::Fraction4096 => "4096ths",
        }
    }

    /// Whether it reads a fraction (so a match is unlikely by chance).
    pub fn fractional(self) -> bool {
        !matches!(self, Self::Whole | Self::Signed)
    }

    pub const ALL: [Encoding; 7] = [
        Self::Whole,
        Self::Signed,
        Self::Sixteenths,
        Self::SignedSixteenths,
        Self::Fraction256,
        Self::SignedFraction256,
        Self::Fraction4096,
    ];
}

/// The width a raw number is read at: a byte, a word, or (for a constant,
/// which has none) the smallest that holds it.
pub fn width_of(raw: i64, declared: Option<u8>) -> u8 {
    declared.unwrap_or(if (0..=0xFF).contains(&raw) { 1 } else { 2 })
}

/// `raw` read with `enc` at `width` bytes, as a model value in 1/65536ths.
/// `None` when the encoding doesn't apply (a sign on a negative constant).
pub fn decode(raw: i64, width: u8, enc: Encoding) -> Option<i64> {
    let signed = || -> Option<i64> {
        if raw < 0 {
            return Some(raw);
        }
        match width {
            1 if raw <= 0xFF => Some(i64::from(raw as u8 as i8)),
            2 if raw <= 0xFFFF => Some(i64::from(raw as u16 as i16)),
            _ => None,
        }
    };
    Some(match enc {
        Encoding::Whole => raw << 16,
        Encoding::Signed => {
            let s = signed()?;
            if s == raw {
                return None;
            }
            s << 16
        }
        Encoding::Sixteenths => raw << 12,
        Encoding::SignedSixteenths => {
            let s = signed()?;
            if s == raw {
                return None;
            }
            s << 12
        }
        Encoding::Fraction256 => raw << 8,
        Encoding::SignedFraction256 => {
            let s = signed()?;
            if s == raw {
                return None;
            }
            s << 8
        }
        Encoding::Fraction4096 => raw << 4,
    })
}

/// Every encoding that reads `raw` as `want` (1/65536ths).
pub fn matches(raw: i64, width: u8, want: i64) -> Vec<Encoding> {
    Encoding::ALL
        .into_iter()
        .filter(|&e| decode(raw, width, e) == Some(want))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: i64 = 1 << 16;

    #[test]
    fn made_up_numbers_read_each_way() {
        // A speed of 1.5 in sixteenths, a negative byte, an 8.8 word.
        assert_eq!(matches(0x18, 1, 3 * ONE / 2), vec![Encoding::Sixteenths]);
        assert_eq!(matches(0xFC, 1, -4 * ONE), vec![Encoding::Signed]);
        assert_eq!(
            matches(0xFB00, 2, -5 * ONE),
            vec![Encoding::SignedFraction256]
        );
        assert_eq!(matches(0x40, 1, ONE / 4), vec![Encoding::Fraction256]);
        assert_eq!(matches(0x98, 1, 0x98 * 16), vec![Encoding::Fraction4096]);
        assert_eq!(matches(7, 1, 7 * ONE), vec![Encoding::Whole]);
        assert_eq!(matches(-3, 1, -3 * ONE), vec![Encoding::Whole]);
        assert!(matches(7, 1, 8 * ONE).is_empty());
        assert_eq!(width_of(0x1234, None), 2);
    }
}
