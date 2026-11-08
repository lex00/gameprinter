//! Branch outcomes read as conditions on places: `STATE == 1`,
//! `PAD & $40 != 0`, `PLAYER_Y >= 208`, `TIMER - 1 == 0`.

use super::absint::{Flags, Guard, Loc, Val};
use crate::codemap::Mn;

/// What a condition is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subj {
    /// A place plus a constant, as the code read it.
    Byte(Loc, i16),
    /// One place minus another, plus a constant.
    Diff(Loc, Loc, i16),
    /// A place's bits under a mask.
    Mask(Loc, u8),
    /// Something else, from these places.
    Other(Vec<Loc>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rel {
    Eq,
    Ne,
    /// Unsigned below.
    Lt,
    /// Unsigned at or above.
    Ge,
    Neg,
    NonNeg,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cond {
    pub subj: Subj,
    pub rel: Rel,
    pub k: u8,
    pub pc: crate::codemap::Va,
}

impl Cond {
    /// The place, when the condition is on one place with no offset or
    /// with one the condition can absorb (`== k` only).
    pub fn byte(&self) -> Option<Loc> {
        match self.subj {
            Subj::Byte(l, 0) => Some(l),
            _ => None,
        }
    }

    /// `place == value`, when it says that.
    pub fn equals(&self) -> Option<(Loc, u8)> {
        match (&self.subj, self.rel) {
            (Subj::Byte(l, off), Rel::Eq) => {
                Some((*l, (i16::from(self.k) - off).rem_euclid(256) as u8))
            }
            _ => None,
        }
    }

    /// `place != value`.
    pub fn differs(&self) -> Option<(Loc, u8)> {
        match (&self.subj, self.rel) {
            (Subj::Byte(l, off), Rel::Ne) => {
                Some((*l, (i16::from(self.k) - off).rem_euclid(256) as u8))
            }
            _ => None,
        }
    }

    pub fn places(&self) -> Vec<Loc> {
        match &self.subj {
            Subj::Byte(l, _) | Subj::Mask(l, _) => vec![*l],
            Subj::Diff(a, b, _) => vec![*a, *b],
            Subj::Other(v) => v.clone(),
        }
    }

    /// As text, with places as `$XX`.
    pub fn text(&self) -> String {
        let l = |l: &Loc| match l.idx {
            None => format!("${:02X}", l.addr),
            Some(r) => format!(
                "${:02X},{}",
                l.addr,
                if r == super::absint::Reg::X { "x" } else { "y" }
            ),
        };
        let off = |o: i16| match o {
            0 => String::new(),
            o if o > 0 => format!(" + {o}"),
            o => format!(" - {}", -o),
        };
        let s = match &self.subj {
            Subj::Byte(a, o) => format!("{}{}", l(a), off(*o)),
            Subj::Diff(a, b, o) => format!("{} - {}{}", l(a), l(b), off(*o)),
            Subj::Mask(a, m) => format!("{} & ${m:02X}", l(a)),
            Subj::Other(v) => format!("f({})", v.iter().map(l).collect::<Vec<_>>().join(", ")),
        };
        let r = match self.rel {
            Rel::Eq => "==",
            Rel::Ne => "!=",
            Rel::Lt => "<",
            Rel::Ge => ">=",
            Rel::Neg => "< 0 (signed)",
            Rel::NonNeg => ">= 0 (signed)",
        };
        if matches!(self.rel, Rel::Neg | Rel::NonNeg) {
            format!("{s} {r}")
        } else {
            format!("{s} {r} {}", self.k)
        }
    }
}

fn subj_of(v: &Val) -> Subj {
    match v.lin() {
        Some((Some(p), None, o)) => Subj::Byte(p, o),
        Some((Some(p), Some(q), o)) => Subj::Diff(p, q, o),
        _ => match v.from {
            Some(l) => Subj::Byte(l, 0),
            None => Subj::Other(v.places().into_iter().collect()),
        },
    }
}

/// The condition a branch outcome states, when it is one on places.
pub fn of(g: &Guard) -> Option<Cond> {
    let t = g.taken;
    let (subj, rel, k) = match &g.test {
        Flags::Cmp { lhs, rhs } => {
            let k = rhs.konst()?;
            let rel = match (g.op, t) {
                (Mn::Beq, true) | (Mn::Bne, false) => Rel::Eq,
                (Mn::Beq, false) | (Mn::Bne, true) => Rel::Ne,
                (Mn::Bcs, true) | (Mn::Bcc, false) => Rel::Ge,
                (Mn::Bcs, false) | (Mn::Bcc, true) => Rel::Lt,
                _ => return None,
            };
            (subj_of(lhs), rel, k)
        }
        Flags::Res { v, src, mask, .. } => {
            let subj = match (mask, src) {
                (Some(m), Some(l)) => Subj::Mask(*l, *m),
                _ => subj_of(v),
            };
            let rel = match (g.op, t) {
                (Mn::Beq, true) | (Mn::Bne, false) => Rel::Eq,
                (Mn::Beq, false) | (Mn::Bne, true) => Rel::Ne,
                (Mn::Bmi, true) | (Mn::Bpl, false) => Rel::Neg,
                (Mn::Bmi, false) | (Mn::Bpl, true) => Rel::NonNeg,
                _ => return None,
            };
            (subj, rel, 0)
        }
        Flags::Carry { op, src } => {
            let l = src.from?;
            let bit = match op {
                Mn::Lsr | Mn::Ror => 0x01,
                Mn::Asl | Mn::Rol => 0x80,
                _ => return None,
            };
            let rel = match (g.op, t) {
                (Mn::Bcs, true) | (Mn::Bcc, false) => Rel::Ne,
                (Mn::Bcs, false) | (Mn::Bcc, true) => Rel::Eq,
                _ => return None,
            };
            (Subj::Mask(l, bit), rel, 0)
        }
        Flags::Unknown => return None,
    };
    Some(Cond {
        subj,
        rel,
        k,
        pc: g.pc,
    })
}
