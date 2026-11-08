//! Fixed-point numbers for game logic.
//!
//! Every number the simulation reads is a [`Fixed`]: a signed 64-bit integer
//! counting 1/65536ths of a unit (16 fraction bits). A model writes one as a
//! TOML integer (`walk_speed = 1`) or, when it has a fraction, as a decimal
//! string (`gravity = "0.25"`). The decimal must be exact: `"0.1"` is refused,
//! since no multiple of 1/65536 equals it. TOML floats are refused too, so no
//! float ever reaches game logic. NES games count subpixels in 1/16ths or
//! 1/256ths, both of which 16 fraction bits hold exactly.
//!
//! The runtime protocol's `fixed(s64)` value carries the same raw integer.

use std::borrow::Cow;
use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Fraction bits in a [`Fixed`].
pub const FRAC_BITS: u32 = 16;
const ONE: i64 = 1 << FRAC_BITS;
/// The largest whole part a [`Fixed`] holds, so that whole numbers written
/// in a model can't overflow when shifted.
const MAX_WHOLE: i64 = (1 << (63 - FRAC_BITS)) - 1;

/// A fixed-point number with [`FRAC_BITS`] fraction bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fixed(i64);

impl Fixed {
    pub const ZERO: Fixed = Fixed(0);

    /// From the raw count of 1/65536ths.
    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> i64 {
        self.0
    }

    /// A whole number. Fails past the range a model may write.
    pub fn from_int(n: i64) -> Result<Self, String> {
        if !(-MAX_WHOLE..=MAX_WHOLE).contains(&n) {
            return Err(format!("{n} is out of range for a fixed-point number"));
        }
        Ok(Self(n << FRAC_BITS))
    }

    /// The whole part when there's no fraction.
    pub fn to_int(self) -> Option<i64> {
        (self.0 % ONE == 0).then_some(self.0 >> FRAC_BITS)
    }

    /// Parses an exact decimal such as `-5`, `0.25` or `128.5`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let err = |why: &str| format!("`{text}` isn't a fixed-point number: {why}");
        let (neg, body) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (whole, frac) = body.split_once('.').unwrap_or((body, ""));
        if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
            return Err(err(
                "write digits, an optional `-` and an optional fraction",
            ));
        }
        if body.contains('.') && (frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit())) {
            return Err(err("the fraction needs digits after the `.`"));
        }
        // 1/65536 has 16 decimal places, so no exact value needs more.
        if frac.len() > 16 {
            return Err(err("more than 16 decimal places"));
        }
        let whole: i64 = whole
            .parse()
            .ok()
            .filter(|w| *w <= MAX_WHOLE)
            .ok_or_else(|| err("out of range"))?;
        let mut raw_frac = 0i64;
        if !frac.is_empty() {
            let digits: u128 = frac.parse().expect("checked digits");
            let scale = 10u128.pow(frac.len() as u32);
            let scaled = digits << FRAC_BITS;
            if !scaled.is_multiple_of(scale) {
                return Err(err(
                    "not a multiple of 1/65536, so it can't be held exactly (game logic is fixed-point)",
                ));
            }
            raw_frac = (scaled / scale) as i64;
        }
        let raw = (whole << FRAC_BITS) + raw_frac;
        Ok(Self(if neg { -raw } else { raw }))
    }
}

impl fmt::Display for Fixed {
    /// The shortest exact decimal: `-5`, `0.25`, `128.5`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let neg = self.0 < 0;
        let abs = self.0.unsigned_abs();
        let whole = abs >> FRAC_BITS;
        let mut frac = abs & (ONE as u64 - 1);
        if neg {
            write!(f, "-")?;
        }
        write!(f, "{whole}")?;
        if frac != 0 {
            write!(f, ".")?;
            while frac != 0 {
                frac *= 10;
                write!(f, "{}", frac >> FRAC_BITS)?;
                frac &= ONE as u64 - 1;
            }
        }
        Ok(())
    }
}

impl Serialize for Fixed {
    /// Whole numbers as TOML integers, others as decimal strings.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.to_int() {
            Some(n) => s.serialize_i64(n),
            None => s.serialize_str(&self.to_string()),
        }
    }
}

impl<'de> Deserialize<'de> for Fixed {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Fixed;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "an integer or an exact decimal string such as \"0.25\"")
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<Fixed, E> {
                Fixed::from_int(n).map_err(E::custom)
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<Fixed, E> {
                let n = i64::try_from(n).map_err(|_| E::custom("out of range"))?;
                Fixed::from_int(n).map_err(E::custom)
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<Fixed, E> {
                Err(E::custom(format!(
                    "{n} is a float; game logic is fixed-point, so write it as a string, like \"{n}\""
                )))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Fixed, E> {
                Fixed::parse(s).map_err(E::custom)
            }
        }
        d.deserialize_any(V)
    }
}

impl schemars::JsonSchema for Fixed {
    fn schema_name() -> Cow<'static, str> {
        "Fixed".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "A fixed-point number with 16 fraction bits: an integer, or an exact decimal string such as \"0.25\". Floats are refused.",
            "oneOf": [
                { "type": "integer" },
                { "type": "string", "pattern": "^-?[0-9]+(\\.[0-9]{1,16})?$" }
            ]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints_exact_decimals() {
        for (text, raw) in [
            ("0", 0),
            ("1", 65536),
            ("-5", -5 * 65536),
            ("0.25", 16384),
            ("128.5", 128 * 65536 + 32768),
            ("-0.5", -32768),
            ("0.0625", 4096),
            ("0.00390625", 256),
            ("0.0000152587890625", 1),
        ] {
            let f = Fixed::parse(text).unwrap();
            assert_eq!(f.raw(), raw, "{text}");
            assert_eq!(f.to_string(), text);
        }
        assert_eq!(Fixed::parse("0.250").unwrap().to_string(), "0.25");
    }

    #[test]
    fn refuses_inexact_and_malformed_numbers() {
        for text in [
            "0.1",
            "1.",
            ".5",
            "",
            "-",
            "1e3",
            "0x10",
            "+1",
            "1.2.3",
            "0.00000000000000001",
        ] {
            assert!(Fixed::parse(text).is_err(), "{text}");
        }
        assert!(Fixed::parse("140737488355328").is_err());
        assert!(Fixed::from_int(i64::MAX).is_err());
    }

    #[derive(Deserialize, Serialize)]
    struct Doc {
        v: Fixed,
    }

    #[test]
    fn toml_integers_and_strings_but_never_floats() {
        let d: Doc = toml::from_str("v = 2").unwrap();
        assert_eq!(d.v, Fixed::from_int(2).unwrap());
        let d: Doc = toml::from_str("v = \"-0.75\"").unwrap();
        assert_eq!(d.v.raw(), -49152);
        let e = toml::from_str::<Doc>("v = 0.25").err().unwrap().to_string();
        assert!(e.contains("fixed-point"), "{e}");
        assert_eq!(
            toml::to_string(&Doc { v: d.v }).unwrap().trim(),
            "v = \"-0.75\""
        );
        assert_eq!(
            toml::to_string(&Doc {
                v: Fixed::from_int(-3).unwrap()
            })
            .unwrap()
            .trim(),
            "v = -3"
        );
    }
}
