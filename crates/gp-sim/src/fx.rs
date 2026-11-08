//! Fixed-point numbers: a signed 64-bit integer counting 1/65536ths, the
//! same raw integer as `gp_model::Fixed` and the runtime protocol's
//! `fixed(s64)`. Arithmetic wraps on overflow, as the originals' does, so
//! every runtime gets the same result for the same inputs.

use std::fmt;

/// Fraction bits in an [`Fx`].
pub const FRAC_BITS: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fx(pub i64);

impl Fx {
    pub const ZERO: Fx = Fx(0);
    pub const ONE: Fx = Fx(1 << FRAC_BITS);

    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> i64 {
        self.0
    }

    pub const fn from_int(n: i64) -> Self {
        Self(n.wrapping_shl(FRAC_BITS))
    }

    /// The whole part, rounded toward negative infinity: the pixel a
    /// position is in.
    pub const fn floor(self) -> i64 {
        self.0 >> FRAC_BITS
    }

    /// The fraction, in 1/65536ths.
    pub const fn frac(self) -> i64 {
        self.0 & ((1 << FRAC_BITS) - 1)
    }

    pub const fn add(self, o: Fx) -> Fx {
        Fx(self.0.wrapping_add(o.0))
    }

    pub const fn sub(self, o: Fx) -> Fx {
        Fx(self.0.wrapping_sub(o.0))
    }

    pub const fn neg(self) -> Fx {
        Fx(self.0.wrapping_neg())
    }

    /// The product, rounded toward negative infinity.
    pub const fn mul(self, o: Fx) -> Fx {
        Fx(((self.0 as i128 * o.0 as i128) >> FRAC_BITS) as i64)
    }

    pub fn min(self, o: Fx) -> Fx {
        if o < self { o } else { self }
    }

    pub fn max(self, o: Fx) -> Fx {
        if o > self { o } else { self }
    }
}

impl fmt::Display for Fx {
    /// The shortest exact decimal, as models write it: `-5`, `0.25`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let one = 1u64 << FRAC_BITS;
        let abs = self.0.unsigned_abs();
        if self.0 < 0 {
            write!(f, "-")?;
        }
        write!(f, "{}", abs >> FRAC_BITS)?;
        let mut frac = abs & (one - 1);
        if frac != 0 {
            write!(f, ".")?;
            while frac != 0 {
                frac *= 10;
                write!(f, "{}", frac >> FRAC_BITS)?;
                frac &= one - 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floors_and_wraps() {
        assert_eq!(Fx::from_int(-5).floor(), -5);
        assert_eq!(Fx(-1).floor(), -1);
        assert_eq!(Fx(0x8_4000).floor(), 8);
        assert_eq!(Fx(0x8_4000).frac(), 0x4000);
        assert_eq!(Fx(i64::MAX).add(Fx(1)), Fx(i64::MIN));
        assert_eq!(Fx::from_int(3).mul(Fx(0x4000)), Fx(0xC000));
        assert_eq!(Fx(-0x4_C000).to_string(), "-4.75");
        assert_eq!(Fx::from_int(128).add(Fx(0x8000)).to_string(), "128.5");
    }
}
