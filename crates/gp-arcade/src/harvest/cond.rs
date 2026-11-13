//! Branch outcomes read as conditions on places: `E397 == 1`,
//! `E38E bit 4 set`, `E4B6+20k - 1 == 0`. The port of gp-nes's
//! `harvest::cond` with the Z80's flags.

use super::absint::{FlagOp, Flags, Guard, Loc, Val};
use super::sem::Cc;

/// What a condition is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subj {
    /// A place plus a constant, as the code read it.
    Byte(Loc, i32),
    /// A place's bits under a mask.
    Mask(Loc, u8),
    /// One bit of a place.
    Bit(Loc, u8),
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
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cond {
    pub subj: Subj,
    pub rel: Rel,
    pub k: u8,
    pub pc: u16,
    /// The flags came from a decrement or increment of the place (`dec
    /// (iy+4); jr nz`): a timer's test.
    pub stepped: bool,
}

impl Cond {
    /// `place == value`, when it says that.
    pub fn equals(&self) -> Option<(Loc, u8)> {
        match (&self.subj, self.rel) {
            (Subj::Byte(l, off), Rel::Eq) => {
                Some((*l, (i32::from(self.k) - off).rem_euclid(256) as u8))
            }
            _ => None,
        }
    }

    /// The place the condition is on.
    pub fn place(&self) -> Option<Loc> {
        match self.subj {
            Subj::Byte(l, _) | Subj::Mask(l, _) | Subj::Bit(l, _) => Some(l),
            Subj::Other(_) => None,
        }
    }

    pub fn text(&self) -> String {
        let rel = match self.rel {
            Rel::Eq => "==",
            Rel::Ne => "!=",
            Rel::Lt => "<",
            Rel::Ge => ">=",
        };
        match &self.subj {
            Subj::Byte(l, 0) => format!("{} {rel} {}", l.hex(), self.k),
            Subj::Byte(l, off) => format!("{} {off:+} {rel} {}", l.hex(), self.k),
            Subj::Mask(l, m) => format!("{} & ${m:02X} {rel} {}", l.hex(), self.k),
            Subj::Bit(l, n) => format!(
                "{} bit {n} {}",
                l.hex(),
                if self.rel == Rel::Eq { "clear" } else { "set" }
            ),
            Subj::Other(ls) => {
                let names: Vec<String> = ls.iter().map(Loc::hex).collect();
                format!("({}) {rel} {}", names.join(" "), self.k)
            }
        }
    }
}

fn subj_of(v: &Val) -> Subj {
    match v.lin() {
        Some((l, 0, off)) => Subj::Byte(l, off),
        _ => match v.from {
            Some(l) => Subj::Byte(l, 0),
            None => Subj::Other(v.places().into_iter().collect()),
        },
    }
}

/// The condition a path took, when the flags say one.
pub fn of(g: &Guard) -> Option<Cond> {
    let mk = |subj, rel, k, stepped| Cond {
        subj,
        rel,
        k,
        pc: g.pc,
        stepped,
    };
    // Z set: the result was 0 (Eq), or the compare's sides were equal.
    let z = match (g.cc, g.taken) {
        (Cc::Z, t) => Some(t),
        (Cc::Nz, t) => Some(!t),
        _ => None,
    };
    let c = match (g.cc, g.taken) {
        (Cc::C, t) => Some(t),
        (Cc::Nc, t) => Some(!t),
        _ => None,
    };
    match &g.test {
        Flags::Cmp { lhs, rhs } => {
            let k = rhs.k8()?;
            let s = subj_of(lhs);
            if let Some(z) = z {
                return Some(mk(s, if z { Rel::Eq } else { Rel::Ne }, k, false));
            }
            c.map(|c| mk(s, if c { Rel::Lt } else { Rel::Ge }, k, false))
        }
        Flags::Res { v, op, src, mask } => {
            let z = z?;
            let rel = if z { Rel::Eq } else { Rel::Ne };
            if let (Some(m), Some(l)) = (mask, src) {
                return Some(mk(Subj::Mask(*l, *m), rel, 0, false));
            }
            let stepped = matches!(op, FlagOp::Inc | FlagOp::Dec);
            let s = match (op, src) {
                (FlagOp::Inc, Some(l)) => Subj::Byte(*l, 1),
                (FlagOp::Dec, Some(l)) => Subj::Byte(*l, -1),
                _ => subj_of(v),
            };
            Some(mk(s, rel, 0, stepped))
        }
        Flags::Bit { n, src, v } => {
            let z = z?;
            let l = src.or_else(|| v.lin().map(|x| x.0))?;
            Some(mk(
                Subj::Bit(l, *n),
                if z { Rel::Eq } else { Rel::Ne },
                0,
                false,
            ))
        }
        Flags::Is { loc, k } => Some(mk(Subj::Byte(*loc, 0), Rel::Eq, *k, false)),
        _ => None,
    }
}
