// license:BSD-3-Clause
// copyright-holders:Aaron Giles
//
// A port of the parts of MAME 0.276's attotime (src/emu/attotime.h and
// attotime.cpp) that the MCR boards use.

//! MAME's time: whole seconds plus attoseconds (10^-18 s). The scheduler's
//! arithmetic has to round exactly as MAME's does, or timeslices end on
//! different cycles.

use std::cmp::Ordering;

pub const ATTOSECONDS_PER_SECOND_SQRT: i64 = 1_000_000_000;
pub const ATTOSECONDS_PER_SECOND: i64 = ATTOSECONDS_PER_SECOND_SQRT * ATTOSECONDS_PER_SECOND_SQRT;
pub const ATTOTIME_MAX_SECONDS: i32 = 1_000_000_000;

/// MAME's HZ_TO_ATTOSECONDS for an integer frequency.
pub const fn hz_to_attoseconds(hz: i64) -> i64 {
    ATTOSECONDS_PER_SECOND / hz
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Attotime {
    pub seconds: i32,
    pub attoseconds: i64,
}

impl Attotime {
    pub const ZERO: Self = Self::new(0, 0);
    pub const NEVER: Self = Self::new(ATTOTIME_MAX_SECONDS, 0);

    pub const fn new(seconds: i32, attoseconds: i64) -> Self {
        Self {
            seconds,
            attoseconds,
        }
    }

    pub const fn is_never(self) -> bool {
        self.seconds >= ATTOTIME_MAX_SECONDS
    }

    /// attotime::from_hz for an unsigned frequency.
    pub const fn from_hz(hz: u32) -> Self {
        if hz > 1 {
            Self::new(0, hz_to_attoseconds(hz as i64))
        } else if hz == 1 {
            Self::new(1, 0)
        } else {
            Self::NEVER
        }
    }

    /// The time as attoseconds, as MAME's as_attoseconds (which assumes it
    /// fits).
    pub const fn as_attoseconds(self) -> i64 {
        if self.seconds == 0 {
            self.attoseconds
        } else if self.seconds == -1 {
            self.attoseconds - ATTOSECONDS_PER_SECOND
        } else if self.seconds > 0 {
            ATTOSECONDS_PER_SECOND
        } else {
            -ATTOSECONDS_PER_SECOND
        }
    }

    /// as_double: seconds plus attoseconds times 1e-18, in that order.
    pub fn as_double(self) -> f64 {
        f64::from(self.seconds) + (self.attoseconds as f64) * 1e-18
    }

    /// operator*= (u32): the attoseconds split into two 9-digit halves.
    pub fn times(self, factor: u32) -> Self {
        if self.seconds >= ATTOTIME_MAX_SECONDS {
            return Self::NEVER;
        }
        if factor == 0 {
            return Self::ZERO;
        }
        let sqrt = ATTOSECONDS_PER_SECOND_SQRT as u64;
        let attohi = (self.attoseconds as u64) / sqrt;
        let attolo = (self.attoseconds as u64) % sqrt;
        let mut temp = attolo * u64::from(factor);
        let reslo = temp % sqrt;
        temp /= sqrt;
        temp += attohi * u64::from(factor);
        let reshi = temp % sqrt;
        temp /= sqrt;
        temp += (self.seconds as u32 as u64) * u64::from(factor);
        if temp >= ATTOTIME_MAX_SECONDS as u64 {
            return Self::NEVER;
        }
        Self::new(
            temp as i32,
            reslo as i64 + (reshi as i64) * ATTOSECONDS_PER_SECOND_SQRT,
        )
    }
}

impl std::ops::Add for Attotime {
    type Output = Self;
    fn add(self, right: Self) -> Self {
        if self.seconds >= ATTOTIME_MAX_SECONDS || right.seconds >= ATTOTIME_MAX_SECONDS {
            return Self::NEVER;
        }
        let mut attos = self.attoseconds + right.attoseconds;
        let mut secs = self.seconds + right.seconds;
        if attos >= ATTOSECONDS_PER_SECOND {
            attos -= ATTOSECONDS_PER_SECOND;
            secs += 1;
        }
        if secs >= ATTOTIME_MAX_SECONDS {
            return Self::NEVER;
        }
        Self::new(secs, attos)
    }
}

impl std::ops::AddAssign for Attotime {
    fn add_assign(&mut self, right: Self) {
        *self = *self + right;
    }
}

impl std::ops::Sub for Attotime {
    type Output = Self;
    fn sub(self, right: Self) -> Self {
        if self.seconds >= ATTOTIME_MAX_SECONDS {
            return Self::NEVER;
        }
        let mut attos = self.attoseconds - right.attoseconds;
        let mut secs = self.seconds - right.seconds;
        if attos < 0 {
            attos += ATTOSECONDS_PER_SECOND;
            secs -= 1;
        }
        Self::new(secs, attos)
    }
}

impl PartialOrd for Attotime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Attotime {
    fn cmp(&self, other: &Self) -> Ordering {
        self.seconds
            .cmp(&other.seconds)
            .then(self.attoseconds.cmp(&other.attoseconds))
    }
}

impl std::fmt::Display for Attotime {
    /// As the oracle's frames.tsv writes it: seconds, a point, 18 digits.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{:018}", self.seconds, self.attoseconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_and_carries_whole_seconds() {
        let a = Attotime::new(0, ATTOSECONDS_PER_SECOND - 5);
        let b = Attotime::new(0, 10);
        assert_eq!(a + b, Attotime::new(1, 5));
        assert_eq!((a + b) - b, a);
        assert!(Attotime::NEVER > a + b);
        assert_eq!(Attotime::NEVER + a, Attotime::NEVER);
    }

    #[test]
    fn hz_rounds_down_as_mame_does() {
        assert_eq!(Attotime::from_hz(30).attoseconds, 33_333_333_333_333_333);
        assert_eq!(Attotime::from_hz(50_000).attoseconds, 20_000_000_000_000);
        assert_eq!(Attotime::from_hz(1), Attotime::new(1, 0));
    }

    #[test]
    fn multiplies_across_the_second() {
        let period = Attotime::from_hz(5_000_000).times(256);
        assert_eq!(period, Attotime::new(0, 51_200_000_000_000));
        let big = Attotime::new(0, 600_000_000_000_000_000).times(3);
        assert_eq!(big, Attotime::new(1, 800_000_000_000_000_000));
        assert_eq!(Attotime::NEVER.times(2), Attotime::NEVER);
    }

    #[test]
    fn prints_as_the_oracle_does() {
        assert_eq!(Attotime::from_hz(30).to_string(), "0.033333333333333333");
    }
}
