//! A static reader of a Z80 program: an abstract interpretation of each
//! routine from the reset and the interrupt handlers, with every call
//! followed in its caller's state, so a value one routine sets for another
//! (a record pointer in IX, an index in A) arrives with what it was. The
//! port of gp-nes's `harvest::absint` to the Z80.
//!
//! A value is a sum, `place << shift + off`, of what a place held when the
//! activation began, or a constant; or a strided address, `base + k *
//! stride`, which a pointer walked through records becomes where a loop's
//! paths meet (`E262` and `E26A` meet as `E262 + 8k`); or a byte or word
//! read from a ROM table at an index; or the low or high byte of a 16-bit
//! value, so `add a,l; ld l,a; ret nc; inc h` (HL plus A by halves) comes
//! back as one 16-bit sum; or unknown, with the places it came from.
//! Places the activation hasn't written read as what they held when it
//! began. Each path carries the branch outcomes that led to it (its
//! guards); where paths meet, only the guards both share stay.
//!
//! Jumps stay in the activation wherever they land, so a tail two routines
//! share (Tapper's `jp $60b8`, the jump through a table) is read in each
//! one's state. What it reads is kept as events, per activation: reads,
//! writes, steps, branch tests, calls, jumps through a register, ports, and
//! reads of the R register. The rules in [`super::rules`] read the events.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::board::Board;
use super::sem::{self, Alu, Block, Cc, Dst, Mem, R8, R16, Rot, Sem, Src};
use crate::dasm;

/// A place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Loc {
    /// One byte.
    At(u16),
    /// A field of a record table: the first record's byte, and the records'
    /// spacing.
    Field { addr: u16, stride: u16 },
    /// A 16-bit variable: the bytes at `a` and `a + 1`, low first.
    Word(u16),
    /// The byte `off` past where the word at `ptr` points.
    Via { ptr: u16, off: i16 },
    /// The byte `off` past where a register points, when the reader doesn't
    /// know where (a routine read out of context).
    Rel { reg: R16, off: i16 },
    /// An input port's value.
    Port(u8),
    /// The R register (the refresh counter): a source of noise.
    Refresh,
}

impl Loc {
    /// The address a place starts at, when it is one.
    pub fn addr(&self) -> Option<u16> {
        match *self {
            Loc::At(a) | Loc::Word(a) | Loc::Field { addr: a, .. } => Some(a),
            _ => None,
        }
    }

    /// The place as hex: `E397`, `E4B2+20k`, `(E0DF)+3`, `(iy+4)`.
    pub fn hex(&self) -> String {
        match *self {
            Loc::At(a) => format!("{a:04X}"),
            Loc::Word(a) => format!("{a:04X}w"),
            Loc::Field { addr, stride } => format!("{addr:04X}+{stride:X}k"),
            Loc::Via { ptr, off } => format!("({ptr:04X}){off:+}"),
            Loc::Rel { reg, off } => format!("({}{off:+})", reg.name()),
            Loc::Port(p) => format!("port {p:02X}"),
            Loc::Refresh => "R".into(),
        }
    }
}

/// What a value is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum K {
    /// `t.0 << t.1 + off`, of what the place held when the activation
    /// began; a constant when `t` is `None`.
    Lin { t: Option<(Loc, u8)>, off: i32 },
    /// `base + k * stride`, k = 0, 1, ...
    Str { base: u16, stride: u16 },
    /// A byte read from a ROM table at `at` plus an index made from a place
    /// shifted left (`by`), when known.
    Tab { at: u16, by: Option<(Loc, u8)> },
    /// A word read from that table: its two bytes put together.
    TabW { at: u16, by: Option<(Loc, u8)> },
    /// A pair whose halves are known apart: high, low.
    Pair(Box<Val>, Box<Val>),
    /// The low byte of a 16-bit value.
    Lo(Box<Val>),
    /// The high byte of a 16-bit value, plus a constant.
    Hi(Box<Val>, i8),
    /// Unknown, from these places.
    Top(BTreeSet<Loc>),
}

/// A value, and the place it was last loaded from unchanged.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Val {
    pub k: K,
    pub from: Option<Loc>,
}

fn gcd(a: u16, b: u16) -> u16 {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl Val {
    pub fn k(c: u16) -> Val {
        Val {
            k: K::Lin {
                t: None,
                off: i32::from(c),
            },
            from: None,
        }
    }
    pub fn top() -> Val {
        Val::taint(BTreeSet::new())
    }
    pub fn taint(t: BTreeSet<Loc>) -> Val {
        Val {
            k: K::Top(t),
            from: None,
        }
    }
    /// What the place held as the activation began.
    pub fn entry(l: Loc) -> Val {
        Val {
            k: K::Lin {
                t: Some((l, 0)),
                off: 0,
            },
            from: Some(l),
        }
    }
    fn of(k: K) -> Val {
        Val { k, from: None }
    }
    /// The constant, if it is one.
    pub fn konst(&self) -> Option<u16> {
        match self.k {
            K::Lin { t: None, off } => Some(off as u16),
            _ => None,
        }
    }
    pub fn k8(&self) -> Option<u8> {
        self.konst().map(|v| v as u8)
    }
    /// `(place, shift, off)` when the value is a place's content shifted
    /// and offset.
    pub fn lin(&self) -> Option<(Loc, u8, i32)> {
        match self.k {
            K::Lin {
                t: Some((l, s)),
                off,
            } => Some((l, s, off)),
            _ => None,
        }
    }
    /// Whether the value is a sum naming the place.
    pub fn names(&self, l: Loc) -> bool {
        match &self.k {
            K::Lin {
                t: Some((p, _)), ..
            } => *p == l,
            K::Tab {
                by: Some((p, _)), ..
            }
            | K::TabW {
                by: Some((p, _)), ..
            } => *p == l,
            K::Pair(h, lo) => h.names(l) || lo.names(l),
            K::Lo(v) | K::Hi(v, _) => v.names(l),
            _ => false,
        }
    }
    /// Every place the value depends on.
    pub fn places(&self) -> BTreeSet<Loc> {
        let mut s: BTreeSet<Loc> = match &self.k {
            K::Lin { t, .. } => t.iter().map(|x| x.0).collect(),
            K::Tab { by, .. } | K::TabW { by, .. } => by.iter().map(|x| x.0).collect(),
            K::Pair(h, l) => h.places().union(&l.places()).copied().collect(),
            K::Lo(v) | K::Hi(v, _) => v.places(),
            K::Str { .. } => BTreeSet::new(),
            K::Top(t) => t.clone(),
        };
        s.extend(self.from);
        s
    }
    fn with_from(mut self, f: Option<Loc>) -> Val {
        self.from = f;
        self
    }
    /// An 8-bit register's value: a constant wraps to a byte.
    fn byte(self) -> Val {
        match self.k {
            K::Lin { t: None, off } => Val::k(u16::from(off as u8)).with_from(self.from),
            _ => self,
        }
    }
    fn join(&self, o: &Val) -> Val {
        if self == o {
            return self.clone();
        }
        let from = if self.from == o.from { self.from } else { None };
        if self.k == o.k {
            return Val {
                k: self.k.clone(),
                from,
            };
        }
        // Two addresses a loop walks: a strided address.
        let st = |v: &Val| match v.k {
            K::Lin { t: None, off } if off >= 0x100 => Some((off as u16, 0u16)),
            K::Str { base, stride } => Some((base, stride)),
            _ => None,
        };
        if let (Some((a, s)), Some((b, t))) = (st(self), st(o)) {
            let base = a.min(b);
            let stride = gcd(gcd(s, t), a.abs_diff(b));
            if stride > 0 && stride <= 0x100 {
                return Val::of(K::Str { base, stride });
            }
        }
        let mut t = self.places();
        t.extend(o.places());
        Val::taint(t).with_from(from)
    }
    /// The value plus a constant (16-bit arithmetic).
    pub fn add(&self, d: i32) -> Val {
        match &self.k {
            K::Lin { t: None, off } => Val::k((off + d) as u16),
            K::Lin { t, off } => Val::of(K::Lin {
                t: *t,
                off: off + d,
            }),
            K::Str { base, stride } => Val::of(K::Str {
                base: (i32::from(*base) + d) as u16,
                stride: *stride,
            }),
            K::Hi(v, a) if (-8..8).contains(&d) => Val::of(K::Hi(v.clone(), a + d as i8)),
            _ => Val::taint(self.places()),
        }
    }
    /// The sum of two values.
    pub fn plus(&self, o: &Val) -> Val {
        // An address plus an index the reader doesn't know: some byte at
        // or after it (a record picked at random).
        for (a, b) in [(self, o), (o, self)] {
            let base = match a.k {
                K::Str { base, .. } => Some(base),
                K::Lin { t: None, off } if off >= 0x100 => Some(off as u16),
                _ => None,
            };
            if let Some(base) = base
                && matches!(b.k, K::Top(_))
            {
                return Val::of(K::Str { base, stride: 1 });
            }
        }
        if let Some(c) = o.konst() {
            return self.add(i32::from(c));
        }
        if let Some(c) = self.konst() {
            return o.add(i32::from(c));
        }
        if let (Some((l, s, a)), Some((m, t, b))) = (self.lin(), o.lin())
            && l == m
            && s == t
        {
            return Val::of(K::Lin {
                t: Some((l, s + 1)),
                off: a + b,
            });
        }
        let mut t = self.places();
        t.extend(o.places());
        Val::taint(t)
    }
    /// The value shifted left once.
    pub fn shl(&self) -> Val {
        match &self.k {
            K::Lin { t: None, off } => Val::k((off << 1) as u16),
            K::Lin {
                t: Some((l, s)),
                off,
            } if *s < 8 => Val::of(K::Lin {
                t: Some((*l, s + 1)),
                off: off << 1,
            }),
            _ => Val::taint(self.places()),
        }
    }
    /// A 16-bit value's high (`hi`) or low byte.
    pub fn half(&self, hi: bool) -> Val {
        if let Some(c) = self.konst() {
            return Val::k(if hi { c >> 8 } else { c & 0xff });
        }
        match &self.k {
            K::Pair(h, l) => {
                if hi {
                    (**h).clone()
                } else {
                    (**l).clone()
                }
            }
            K::Top(t) => Val::taint(t.clone()),
            // A byte a table gave: its high half is not known.
            K::Tab { .. } if hi => Val::taint(self.places()),
            K::Tab { .. } => self.clone(),
            _ => Val::of(if hi {
                K::Hi(Box::new(self.clone()), 0)
            } else {
                K::Lo(Box::new(self.clone()))
            }),
        }
    }
    /// A pair from its halves.
    pub fn pair(hi: Val, lo: Val) -> Val {
        if let (Some(h), Some(l)) = (hi.konst(), lo.konst()) {
            return Val::k((h << 8) | (l & 0xff));
        }
        match (&hi.k, &lo.k) {
            (K::Hi(a, 0), K::Lo(b)) if a == b => return (**a).clone(),
            (K::Tab { at: ah, by: bh }, K::Tab { at: al, by: bl })
                if *ah == al.wrapping_add(1) && bh == bl =>
            {
                return Val::of(K::TabW { at: *al, by: *bl });
            }
            _ => {}
        }
        // Two bytes of one 16-bit variable, as they were.
        if let (Some((Loc::At(h), 0, 0)), Some((Loc::At(l), 0, 0))) = (hi.lin(), lo.lin())
            && h == l.wrapping_add(1)
        {
            return Val::entry(Loc::Word(l));
        }
        // A byte zero-extended.
        if hi.konst() == Some(0)
            && matches!(lo.k, K::Lin { t: Some((_, 0..=1)), off } if (0..0x100).contains(&off))
        {
            return lo;
        }
        if hi.konst() == Some(0) && matches!(lo.k, K::Tab { .. }) {
            return lo;
        }
        Val::of(K::Pair(Box::new(hi), Box::new(lo)))
    }
}

impl Val {
    /// The value with each place it names put through `f`.
    pub fn map_locs(&self, f: &dyn Fn(Loc) -> Loc) -> Val {
        let by = |b: &Option<(Loc, u8)>| b.map(|(l, s)| (f(l), s));
        let k = match &self.k {
            K::Lin { t, off } => K::Lin {
                t: by(t),
                off: *off,
            },
            K::Str { .. } => self.k.clone(),
            K::Tab { at, by: b } => K::Tab { at: *at, by: by(b) },
            K::TabW { at, by: b } => K::TabW { at: *at, by: by(b) },
            K::Pair(h, l) => K::Pair(Box::new(h.map_locs(f)), Box::new(l.map_locs(f))),
            K::Lo(v) => K::Lo(Box::new(v.map_locs(f))),
            K::Hi(v, a) => K::Hi(Box::new(v.map_locs(f)), *a),
            K::Top(t) => K::Top(t.iter().map(|l| f(*l)).collect()),
        };
        Val {
            k,
            from: self.from.map(f),
        }
    }
}

impl Flags {
    pub fn map_locs(&self, f: &dyn Fn(Loc) -> Loc) -> Flags {
        match self {
            Flags::Cmp { lhs, rhs } => Flags::Cmp {
                lhs: lhs.map_locs(f),
                rhs: rhs.map_locs(f),
            },
            Flags::Res { v, op, src, mask } => Flags::Res {
                v: v.map_locs(f),
                op: *op,
                src: src.map(f),
                mask: *mask,
            },
            Flags::Bit { n, v, src } => Flags::Bit {
                n: *n,
                v: v.map_locs(f),
                src: src.map(f),
            },
            Flags::Carry { sum, w } => Flags::Carry {
                sum: sum.map_locs(f),
                w: w.map_locs(f),
            },
            Flags::Is { loc, k } => Flags::Is {
                loc: f(*loc),
                k: *k,
            },
            Flags::Unknown | Flags::B => self.clone(),
        }
    }
}

impl Guard {
    pub fn map_locs(&self, f: &dyn Fn(Loc) -> Loc) -> Guard {
        Guard {
            test: self.test.map_locs(f),
            ..self.clone()
        }
    }
}

fn map_guards(g: &BTreeSet<Guard>, f: &dyn Fn(Loc) -> Loc) -> BTreeSet<Guard> {
    g.iter().map(|x| x.map_locs(f)).collect()
}

impl Ev {
    /// The event with each place it names put through `f`.
    pub fn map_locs(&self, f: &dyn Fn(Loc) -> Loc) -> Ev {
        let by = |b: &Option<(Loc, u8)>| b.map(|(l, s)| (f(l), s));
        match self {
            Ev::Read {
                pc,
                loc,
                val,
                fresh,
            } => Ev::Read {
                pc: *pc,
                loc: f(*loc),
                val: val.map_locs(f),
                fresh: *fresh,
            },
            Ev::Table { pc, at, by: b } => Ev::Table {
                pc: *pc,
                at: *at,
                by: by(b),
            },
            Ev::Write {
                pc,
                loc,
                val,
                guards,
                added,
            } => Ev::Write {
                pc: *pc,
                loc: f(*loc),
                val: val.map_locs(f),
                guards: map_guards(guards, f),
                added: *added,
            },
            Ev::Step {
                pc,
                loc,
                by: d,
                guards,
            } => Ev::Step {
                pc: *pc,
                loc: f(*loc),
                by: *d,
                guards: map_guards(guards, f),
            },
            Ev::Branch {
                pc,
                cc,
                test,
                guards,
            } => Ev::Branch {
                pc: *pc,
                cc: *cc,
                test: test.map_locs(f),
                guards: map_guards(guards, f),
            },
            Ev::Call {
                pc,
                to,
                act,
                guards,
            } => Ev::Call {
                pc: *pc,
                to: *to,
                act: *act,
                guards: map_guards(guards, f),
            },
            Ev::Indirect { pc, target, to } => Ev::Indirect {
                pc: *pc,
                target: target.map_locs(f),
                to: to
                    .iter()
                    .map(|(t, c)| (*t, c.map(|(l, k)| (f(l), k))))
                    .collect(),
            },
            Ev::Out { pc, port, val } => Ev::Out {
                pc: *pc,
                port: *port,
                val: val.map_locs(f),
            },
            Ev::Block { pc, dst, src, len } => Ev::Block {
                pc: *pc,
                dst: dst.map_locs(f),
                src: src.map_locs(f),
                len: len.map_locs(f),
            },
            Ev::In { .. } | Ev::Walk { .. } | Ev::Decimal { .. } => self.clone(),
        }
    }
}

/// What the flags of the last instruction that set them describe.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Flags {
    Unknown,
    /// `cp` and `sub`: Z and C from `lhs - rhs`, unsigned.
    Cmp {
        lhs: Val,
        rhs: Val,
    },
    /// Z (and S) from a result; for `and n`, the masked place and the mask.
    Res {
        v: Val,
        op: FlagOp,
        src: Option<Loc>,
        mask: Option<u8>,
    },
    /// `bit n`: Z set when the bit is clear.
    Bit {
        n: u8,
        v: Val,
        src: Option<Loc>,
    },
    /// The carry of a 16-bit sum `w + x` made by halves (`add a,l`).
    Carry {
        sum: Val,
        w: Val,
    },
    /// A jump table's case: the place holds `k`.
    Is {
        loc: Loc,
        k: u8,
    },
    /// `djnz`: B after its decrement.
    B,
}

/// The instruction that set the flags of a [`Flags::Res`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlagOp {
    Inc,
    Dec,
    And,
    Or,
    Xor,
    Add,
    Shift,
    Other,
}

/// One branch outcome a path took.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Guard {
    pub pc: u16,
    pub cc: Cc,
    pub taken: bool,
    pub test: Flags,
}

/// The machine state of one path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct St {
    pub a: Val,
    pub f: Flags,
    pub bc: Val,
    pub de: Val,
    pub hl: Val,
    pub ix: Val,
    pub iy: Val,
    /// The second set: A', F', BC', DE', HL'.
    pub alt: (Val, Flags, Val, Val, Val),
    pub mem: BTreeMap<Loc, Val>,
    pub stack: Vec<Val>,
    pub guards: BTreeSet<Guard>,
    /// Places every path to here has written.
    pub wrote: BTreeSet<Loc>,
    /// Reads of places this activation hasn't written read as unknown (a
    /// routine read out of context).
    pub general: bool,
}

impl St {
    pub fn fresh() -> St {
        St {
            a: Val::top(),
            f: Flags::Unknown,
            bc: Val::top(),
            de: Val::top(),
            hl: Val::top(),
            ix: Val::top(),
            iy: Val::top(),
            alt: (
                Val::top(),
                Flags::Unknown,
                Val::top(),
                Val::top(),
                Val::top(),
            ),
            mem: BTreeMap::new(),
            stack: Vec::new(),
            guards: BTreeSet::new(),
            wrote: BTreeSet::new(),
            general: false,
        }
    }

    pub fn general() -> St {
        St {
            general: true,
            ..St::fresh()
        }
    }

    fn join(&self, o: &St) -> St {
        let mut mem = BTreeMap::new();
        let keys: BTreeSet<Loc> = self.mem.keys().chain(o.mem.keys()).copied().collect();
        for k in keys {
            let a = self
                .mem
                .get(&k)
                .cloned()
                .unwrap_or_else(|| self.unwritten(k));
            let b = o.mem.get(&k).cloned().unwrap_or_else(|| o.unwritten(k));
            // Two contents of one place meet as the place's own symbol:
            // what it holds now, whichever way the code came.
            let j = if a.k == b.k {
                a.join(&b)
            } else if self.general || o.general {
                Val::taint([k].into())
            } else {
                Val::entry(k).with_from(None)
            };
            // The place's own symbol is what a read of it gives anyway:
            // the map keeps only what the paths know.
            if j.k == Val::entry(k).k || (j.k == K::Top([k].into())) {
                continue;
            }
            mem.insert(k, j);
        }
        let stack = if self.stack.len() == o.stack.len() {
            self.stack
                .iter()
                .zip(&o.stack)
                .map(|(a, b)| a.join(b))
                .collect()
        } else {
            Vec::new()
        };
        let fj = |a: &Flags, b: &Flags| if a == b { a.clone() } else { Flags::Unknown };
        St {
            a: self.a.join(&o.a),
            f: fj(&self.f, &o.f),
            bc: self.bc.join(&o.bc),
            de: self.de.join(&o.de),
            hl: self.hl.join(&o.hl),
            ix: self.ix.join(&o.ix),
            iy: self.iy.join(&o.iy),
            alt: (
                self.alt.0.join(&o.alt.0),
                fj(&self.alt.1, &o.alt.1),
                self.alt.2.join(&o.alt.2),
                self.alt.3.join(&o.alt.3),
                self.alt.4.join(&o.alt.4),
            ),
            mem,
            stack,
            guards: self.guards.intersection(&o.guards).cloned().collect(),
            wrote: self.wrote.intersection(&o.wrote).copied().collect(),
            general: self.general || o.general,
        }
    }

    /// The join, with what still differs made unknown: the state after a
    /// loop that wouldn't settle. What both sides agree on stays.
    fn widen(&self, o: &St) -> St {
        let mut j = self.join(o);
        let pairs = [
            (&mut j.a, &self.a, &o.a),
            (&mut j.bc, &self.bc, &o.bc),
            (&mut j.de, &self.de, &o.de),
            (&mut j.hl, &self.hl, &o.hl),
            (&mut j.ix, &self.ix, &o.ix),
            (&mut j.iy, &self.iy, &o.iy),
        ];
        for (v, a, b) in pairs {
            if a != b && !matches!(v.k, K::Str { .. } | K::Top(_)) {
                *v = Val::taint(v.places());
            }
        }
        for (k, v) in j.mem.iter_mut() {
            if self.mem.get(k) != o.mem.get(k) && !matches!(v.k, K::Top(_)) && !v.names(*k) {
                *v = Val::taint(v.places());
            }
        }
        j
    }

    fn unwritten(&self, l: Loc) -> Val {
        if self.general {
            Val::taint([l].into()).with_from(Some(l))
        } else {
            Val::entry(l)
        }
    }

    /// Writes a place. A value the reader can't name, or one that names
    /// the place itself, is kept as the place's own symbol. Every other
    /// value that names the place's old content is forgotten first.
    fn set(&mut self, loc: Loc, v: Val) -> Val {
        let keep = !matches!(v.k, K::Top(_)) && !v.names(loc);
        let copied = v.from.filter(|f| *f != loc);
        self.forget(loc);
        let stored = if keep {
            v.with_from(copied)
        } else if self.general {
            Val::taint([loc].into()).with_from(copied)
        } else {
            Val::entry(loc).with_from(copied)
        };
        self.mem.insert(loc, stored.clone());
        self.wrote.insert(loc);
        stored
    }

    /// The place changes: values that name its old content don't any more,
    /// and nor do the 16-bit variables it is a byte of.
    fn forget(&mut self, loc: Loc) {
        let mut gone = vec![loc];
        if let Loc::At(a) = loc {
            gone.push(Loc::Word(a));
            gone.push(Loc::Word(a.wrapping_sub(1)));
        }
        if let Loc::Word(a) = loc {
            gone.push(Loc::At(a));
            gone.push(Loc::At(a.wrapping_add(1)));
        }
        for g in &gone[1..] {
            self.mem.remove(g);
        }
        let fix = |v: &mut Val| {
            for g in &gone {
                if v.names(*g) {
                    *v = Val::taint(v.places());
                } else if v.from == Some(*g) {
                    v.from = None;
                }
            }
        };
        for v in [
            &mut self.a,
            &mut self.bc,
            &mut self.de,
            &mut self.hl,
            &mut self.ix,
            &mut self.iy,
        ] {
            fix(v);
        }
        self.mem.values_mut().for_each(fix);
        self.stack.iter_mut().for_each(fix);
        match &mut self.f {
            Flags::Cmp { lhs, rhs } => {
                fix(lhs);
                fix(rhs);
            }
            Flags::Res { v, .. } | Flags::Bit { v, .. } => fix(v),
            Flags::Carry { sum, w } => {
                fix(sum);
                fix(w);
            }
            _ => {}
        }
    }

    pub fn get16(&self, r: R16) -> Val {
        match r {
            R16::Bc => self.bc.clone(),
            R16::De => self.de.clone(),
            R16::Hl => self.hl.clone(),
            R16::Ix => self.ix.clone(),
            R16::Iy => self.iy.clone(),
            R16::Af => Val::pair(self.a.clone(), Val::top()),
            R16::Sp => Val::top(),
        }
    }

    pub fn set16(&mut self, r: R16, v: Val) {
        match r {
            R16::Bc => self.bc = v,
            R16::De => self.de = v,
            R16::Hl => self.hl = v,
            R16::Ix => self.ix = v,
            R16::Iy => self.iy = v,
            R16::Af => {
                self.a = v.half(true).byte();
                self.f = Flags::Unknown;
            }
            R16::Sp => {}
        }
    }

    pub fn get8(&self, r: R8) -> Val {
        match r {
            R8::A => self.a.clone(),
            R8::I => Val::top(),
            R8::R => Val::entry(Loc::Refresh),
            _ => {
                let (p, hi) = r.pair().expect("a half");
                self.get16(p).half(hi)
            }
        }
    }

    pub fn set8(&mut self, r: R8, v: Val) {
        let v = v.byte();
        match r {
            R8::A => self.a = v,
            R8::I | R8::R => {}
            _ => {
                let (p, hi) = r.pair().expect("a half");
                let old = self.get16(p);
                let w = if hi {
                    Val::pair(v, old.half(false))
                } else {
                    Val::pair(old.half(true), v)
                };
                self.set16(p, w);
            }
        }
    }
}

/// What the reader saw one instruction do, in one activation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ev {
    Read {
        pc: u16,
        loc: Loc,
        val: Val,
        /// No path to here wrote the place in this activation.
        fresh: bool,
    },
    /// A byte read from a ROM table at an index.
    Table {
        pc: u16,
        at: u16,
        by: Option<(Loc, u8)>,
    },
    Write {
        pc: u16,
        loc: Loc,
        val: Val,
        guards: BTreeSet<Guard>,
        /// The flags at the store come from an add or a subtract: the value
        /// is a sum (`add a,(ix+3); ld (ix+3),a`).
        added: bool,
    },
    /// A place set to itself plus or minus a constant (`inc (hl)`, or `ld
    /// a,(x); inc a; ld (x),a`).
    Step {
        pc: u16,
        loc: Loc,
        by: i8,
        guards: BTreeSet<Guard>,
    },
    Branch {
        pc: u16,
        cc: Cc,
        test: Flags,
        guards: BTreeSet<Guard>,
    },
    Call {
        pc: u16,
        to: u16,
        act: usize,
        guards: BTreeSet<Guard>,
    },
    /// A jump through a register (or a `ret` to a pushed address), with the
    /// value and where the reader sent it.
    Indirect {
        pc: u16,
        target: Val,
        to: Vec<(u16, Option<(Loc, u8)>)>,
    },
    In {
        pc: u16,
        port: Option<u8>,
    },
    Out {
        pc: u16,
        port: Option<u8>,
        val: Val,
    },
    /// `ldir` and the like: `len` bytes copied from `src` to `dst`.
    Block {
        pc: u16,
        dst: Val,
        src: Val,
        len: Val,
    },
    /// A loop whose paths met at `head` with a register walking records.
    Walk {
        head: u16,
        reg: R16,
        base: u16,
        stride: u16,
        /// B as the loop was first entered (a `djnz` count).
        count: Option<u8>,
    },
    /// `daa` after an add: a decimal digit.
    Decimal {
        pc: u16,
    },
}

impl Ev {
    /// The instruction it is about (a walk's loop head).
    pub fn pc(&self) -> u16 {
        match self {
            Ev::Read { pc, .. }
            | Ev::Table { pc, .. }
            | Ev::Write { pc, .. }
            | Ev::Step { pc, .. }
            | Ev::Branch { pc, .. }
            | Ev::Call { pc, .. }
            | Ev::Indirect { pc, .. }
            | Ev::In { pc, .. }
            | Ev::Out { pc, .. }
            | Ev::Block { pc, .. }
            | Ev::Decimal { pc } => *pc,
            Ev::Walk { head, .. } => *head,
        }
    }
}

/// One routine run in one caller state.
#[derive(Debug, Clone)]
pub struct Act {
    pub start: u16,
    pub general: bool,
    /// Run as an interrupt handler.
    pub interrupt: bool,
    pub events: BTreeSet<Ev>,
    /// Who called it: (activation, call's pc, the caller's guards there).
    pub callers: BTreeSet<(usize, u16, BTreeSet<Guard>)>,
    pub returns: bool,
    /// While it is read: each instruction's events from its last visit
    /// (a later visit's state is the join of the earlier ones, so it says
    /// all they did), and each loop head's walks.
    at_pc: BTreeMap<u16, Vec<Ev>>,
    walks: BTreeMap<(u16, R16), Ev>,
    /// The instructions it read.
    pub size: usize,
}

/// Activations per routine before the reader stops telling callers apart.
const CONTEXTS: usize = 8;
/// Instructions a routine that calls nothing has at most to keep its
/// callers apart for longer.
const LEAF_SIZE: usize = 24;
/// The same for a short routine that calls nothing.
const LEAF_CONTEXTS: usize = 256;
/// Calls deep before the same.
const DEPTH: usize = 24;
/// Visits of one instruction in one activation before its state widens.
const VISITS: usize = 8;
/// Instructions one activation reads before it stops.
const STEPS: usize = 40_000;
/// How far from its table a jump table's entry may point.
const TABLE_REACH: u16 = 0x1000;
/// Entries read from a jump table whose index's values aren't known.
const TABLE_MAX: u16 = 48;

/// One decoded instruction: its semantics, length and text.
#[derive(Debug, Clone)]
pub struct Insn {
    pub pc: u16,
    pub sem: Sem,
    pub len: u8,
    pub text: String,
}

pub struct Reader<'a> {
    pub board: &'a Board,
    pub acts: Vec<Act>,
    memo: HashMap<(u16, St), (usize, Option<St>)>,
    general: BTreeMap<u16, (usize, Option<St>)>,
    per_routine: BTreeMap<u16, usize>,
    stack: Vec<u16>,
    insns: std::cell::RefCell<HashMap<u16, Insn>>,
    /// I's value (`ld i,a`), the IM mode set, and the vectors written to
    /// the CTC's channel 0.
    pub i_reg: BTreeSet<u8>,
    pub im: BTreeSet<u8>,
    pub ctc_vectors: BTreeSet<u8>,
    /// Jump-table edges: (the jump's pc, a target).
    pub tables: BTreeSet<(u16, u16)>,
    /// Call and restart targets reached.
    pub called: BTreeSet<u16>,
    /// From the pass before: the values each place is written with (a
    /// table's index bounds its entries), and the constants each 16-bit
    /// variable is set to (a jump through it goes to each).
    pub index_values: BTreeMap<Loc, BTreeSet<u8>>,
    pub hints: BTreeMap<u16, BTreeSet<u16>>,
    /// Routine starts the code map has (no instruction of a table runs past
    /// one).
    pub starts: BTreeSet<u16>,
    /// Places the interrupt handlers write: an interrupt can change them
    /// between any two instructions, so a read of one is what it holds now,
    /// never what the path last wrote (a wait for the frame counter).
    pub volatile: BTreeSet<Loc>,
    /// Routines read so far that call nothing, and those of them that
    /// touch no memory either.
    leaves: BTreeSet<u16>,
    pure: BTreeSet<u16>,
    /// The instruction being read stores A (`ld (..),a`): a store whose
    /// flags can say A is a sum.
    store_from_sum: bool,
}

impl<'a> Reader<'a> {
    pub fn new(board: &'a Board) -> Self {
        Reader {
            board,
            acts: Vec::new(),
            memo: HashMap::new(),
            general: BTreeMap::new(),
            per_routine: BTreeMap::new(),
            stack: Vec::new(),
            insns: Default::default(),
            i_reg: BTreeSet::new(),
            im: BTreeSet::new(),
            ctc_vectors: BTreeSet::new(),
            tables: BTreeSet::new(),
            called: BTreeSet::new(),
            index_values: BTreeMap::new(),
            hints: BTreeMap::new(),
            starts: BTreeSet::new(),
            volatile: BTreeSet::new(),
            leaves: BTreeSet::new(),
            pure: BTreeSet::new(),
            store_from_sum: false,
        }
    }

    /// What the activations so far write: run after the handlers, the
    /// places an interrupt changes.
    pub fn writes_so_far(&self) -> BTreeSet<Loc> {
        let mut out = BTreeSet::new();
        for a in &self.acts {
            for e in &a.events {
                if let Ev::Write { loc, .. } | Ev::Step { loc, .. } = e {
                    out.insert(*loc);
                    if let Loc::Word(w) = loc {
                        out.insert(Loc::At(*w));
                        out.insert(Loc::At(w.wrapping_add(1)));
                    }
                }
            }
        }
        out
    }

    /// The instruction at `pc`.
    pub fn insn(&self, pc: u16) -> Insn {
        if let Some(i) = self.insns.borrow().get(&pc) {
            return i.clone();
        }
        let b = self.board.bytes(pc);
        let d = dasm::decode(&b, pc);
        let i = Insn {
            pc,
            sem: sem::decode(&b, pc),
            len: d.len,
            text: d.text,
        };
        self.insns.borrow_mut().insert(pc, i.clone());
        i
    }

    /// The IM 2 handlers the code set up: the words at `I * 256 + vector +
    /// 2n` for each CTC channel n, with the channel.
    pub fn im2_handlers(&self) -> Vec<(u16, u8)> {
        let mut out = Vec::new();
        if !self.im.contains(&2) {
            return out;
        }
        for &i in &self.i_reg {
            for &v in &self.ctc_vectors {
                for ch in 0..4u8 {
                    let at = (u16::from(i) << 8) | u16::from(v.wrapping_add(ch * 2));
                    if !self.board.is_rom(at) {
                        continue;
                    }
                    let h = self.board.word(at);
                    if self.board.is_rom(h) && !out.iter().any(|x: &(u16, u8)| x.0 == h) {
                        out.push((h, ch));
                    }
                }
            }
        }
        out
    }

    /// Reads from the reset with nothing known.
    pub fn run_reset(&mut self) {
        self.call(0, St::fresh(), None);
    }

    /// Reads each interrupt handler with nothing known.
    pub fn run_interrupts(&mut self, handlers: &[u16]) {
        for &h in handlers {
            let n = self.acts.len();
            self.call(h, St::fresh(), None);
            if let Some(a) = self.acts.get_mut(n) {
                a.interrupt = true;
            }
        }
    }

    /// Reads, out of context, each start no path read.
    pub fn run_entries(&mut self, starts: &[u16]) {
        let read: BTreeSet<u16> = self.acts.iter().map(|a| a.start).collect();
        for &s in starts {
            if !read.contains(&s) {
                self.call(s, St::general(), None);
            }
        }
    }

    /// Runs `start` in `entry`'s state for the caller (`caller`: its
    /// activation, the call's pc and its guards). The activation and the
    /// state it returns with, if it returns.
    pub fn call(
        &mut self,
        start: u16,
        entry: St,
        caller: Option<(usize, u16, BTreeSet<Guard>)>,
    ) -> (usize, Option<St>) {
        self.called.insert(start);
        // A routine that touches no memory and calls nothing (HL += A) is
        // read on the registers alone, however many callers it has: its
        // exit is the caller's state with its registers.
        if self.pure.contains(&start) && !entry.general {
            let mut regs = entry.clone();
            regs.mem.clear();
            regs.wrote.clear();
            regs.guards.clear();
            regs.stack.clear();
            let (id, exit) = self.call_keyed(start, regs, caller);
            let exit = exit.map(|e| {
                let mut n = entry.clone();
                n.a = e.a;
                n.f = e.f;
                n.bc = e.bc;
                n.de = e.de;
                n.hl = e.hl;
                n.ix = e.ix;
                n.iy = e.iy;
                n.alt = e.alt;
                n
            });
            return (id, exit);
        }
        self.call_keyed(start, entry, caller)
    }

    fn call_keyed(
        &mut self,
        start: u16,
        entry: St,
        caller: Option<(usize, u16, BTreeSet<Guard>)>,
    ) -> (usize, Option<St>) {
        let mut key = entry;
        key.guards.clear();
        key.stack.clear();
        // A routine that calls nothing is cheap to read again: it keeps
        // its callers apart for longer (a helper such as HL += A, which
        // every table read goes through).
        let limit = if self.leaves.contains(&start) {
            LEAF_CONTEXTS
        } else {
            CONTEXTS
        };
        let general = key.general
            || self.stack.len() >= DEPTH
            || self.per_routine.get(&start).copied().unwrap_or(0) >= limit;
        if self.stack.contains(&start) {
            // Recursion: what it returns is not known.
            let mut out = key.clone();
            out.a = Val::top();
            out.bc = Val::top();
            out.de = Val::top();
            out.hl = Val::top();
            out.f = Flags::Unknown;
            return (usize::MAX, Some(out));
        }
        if general {
            if let Some((id, exit)) = self.general.get(&start).cloned() {
                if let Some(c) = caller {
                    self.acts[id].callers.insert(c);
                }
                return (id, exit.map(|e| over(&key, e)));
            }
        } else if let Some((id, exit)) = self.memo.get(&(start, key.clone())).cloned() {
            if let Some(c) = caller {
                self.acts[id].callers.insert(c);
            }
            return (id, exit);
        }
        let id = self.acts.len();
        self.acts.push(Act {
            start,
            general,
            interrupt: false,
            events: BTreeSet::new(),
            callers: caller.into_iter().collect(),
            returns: false,
            at_pc: BTreeMap::new(),
            walks: BTreeMap::new(),
            size: 0,
        });
        self.stack.push(start);
        let run_with = if general { St::general() } else { key.clone() };
        if general {
            // Placed before the body so a call back into it finds it.
            self.general.insert(start, (id, None));
        } else {
            *self.per_routine.entry(start).or_default() += 1;
        }
        let exit = self.body(id, start, run_with);
        self.stack.pop();
        let a = &mut self.acts[id];
        a.returns = exit.is_some();
        if !a
            .at_pc
            .values()
            .flatten()
            .any(|e| matches!(e, Ev::Call { .. }))
            && a.size <= LEAF_SIZE
        {
            self.leaves.insert(start);
            let touches = a.at_pc.values().flatten().any(|e| {
                matches!(
                    e,
                    Ev::Read { .. }
                        | Ev::Write { .. }
                        | Ev::Step { .. }
                        | Ev::Table { .. }
                        | Ev::Block { .. }
                        | Ev::Indirect { .. }
                        | Ev::In { .. }
                        | Ev::Out { .. }
                )
            });
            if !touches && a.returns {
                self.pure.insert(start);
            }
        }
        let at_pc = std::mem::take(&mut a.at_pc);
        let walks = std::mem::take(&mut a.walks);
        a.events = at_pc
            .into_values()
            .flatten()
            .chain(walks.into_values())
            .collect();
        if general {
            self.general.insert(start, (id, exit.clone()));
            (id, exit.map(|e| over(&key, e)))
        } else {
            self.memo.insert((start, key), (id, exit.clone()));
            (id, exit)
        }
    }

    fn body(&mut self, id: usize, start: u16, entry: St) -> Option<St> {
        let mut at: BTreeMap<u16, (St, usize)> = BTreeMap::new();
        let mut work: BTreeSet<u16> = BTreeSet::new();
        at.insert(start, (entry, 0));
        work.insert(start);
        let mut exit: Option<St> = None;
        let mut steps = 0;
        while let Some(pc) = work.pop_first() {
            steps += 1;
            if steps > STEPS {
                break;
            }
            let st = at[&pc].0.clone();
            if let Some(a) = self.acts.get_mut(id) {
                a.at_pc.remove(&pc);
            }
            for (next, ns) in self.step(id, pc, st, &mut exit) {
                if !self.board.is_rom(next) {
                    continue;
                }
                match at.get_mut(&next) {
                    None => {
                        at.insert(next, (ns, 0));
                        work.insert(next);
                    }
                    Some((old, visits)) => {
                        let j = if *visits >= VISITS {
                            old.widen(&ns)
                        } else {
                            old.join(&ns)
                        };
                        if j != *old && next <= pc {
                            // A loop's back edge: a pair that became a walk
                            // through records.
                            for (reg, a, b) in [
                                (R16::Ix, &old.ix, &j.ix),
                                (R16::Iy, &old.iy, &j.iy),
                                (R16::Hl, &old.hl, &j.hl),
                                (R16::De, &old.de, &j.de),
                            ] {
                                if let K::Str { base, stride } = b.k
                                    && a.k != b.k
                                {
                                    let mut count = old.bc.half(true).k8();
                                    if let Some(act) = self.acts.get_mut(id) {
                                        // The count is B as the loop was
                                        // first entered: a later meeting
                                        // keeps it.
                                        if let Some(Ev::Walk { count: c, .. }) =
                                            act.walks.get(&(next, reg))
                                        {
                                            count = count.or(*c);
                                        }
                                        act.walks.insert(
                                            (next, reg),
                                            Ev::Walk {
                                                head: next,
                                                reg,
                                                base,
                                                stride,
                                                count,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        if j != *old {
                            *old = j;
                            *visits += 1;
                            work.insert(next);
                        }
                    }
                }
            }
        }
        if let Some(a) = self.acts.get_mut(id) {
            a.size = at.len();
        }
        exit
    }

    fn ev(&mut self, id: usize, e: Ev) {
        if let Some(a) = self.acts.get_mut(id) {
            a.at_pc.entry(e.pc()).or_default().push(e);
        }
    }

    /// The address a memory operand names, as a value.
    fn address(&self, m: Mem, st: &St) -> Val {
        match m {
            Mem::Abs(n) => Val::k(n),
            Mem::Pair(r) => st.get16(r),
            Mem::Idx(r, d) => st.get16(r).add(i32::from(d)),
        }
    }

    /// Where an address value points.
    fn place(&self, m: Mem, a: &Val) -> Place {
        let reg = match m {
            Mem::Abs(_) => R16::Hl,
            Mem::Pair(r) | Mem::Idx(r, _) => r,
        };
        let disp = match m {
            Mem::Idx(_, d) => i16::from(d),
            _ => 0,
        };
        match &a.k {
            K::Lin { t: None, off } => {
                let x = *off as u16;
                if self.board.is_rom(x) {
                    Place::Rom(x)
                } else {
                    Place::Ram(Loc::At(x))
                }
            }
            K::Str { base, stride } => {
                if self.board.is_rom(*base) {
                    Place::Table(*base, None)
                } else {
                    Place::Ram(Loc::Field {
                        addr: *base,
                        stride: *stride,
                    })
                }
            }
            K::Lin {
                t: Some((l, s)),
                off,
            } => {
                let base = *off as u16;
                if (0..0x10000).contains(off) && self.board.is_rom(base) && *off > 0 {
                    Place::Table(base, Some((*l, *s)))
                } else if let (Loc::Word(p), 0) = (l, s) {
                    Place::Ram(Loc::Via {
                        ptr: *p,
                        off: *off as i16,
                    })
                } else if (0x100..0x10000).contains(off) && *s < 8 {
                    // A RAM address plus a byte shifted: a table in RAM
                    // indexed by it.
                    Place::Ram(Loc::Field {
                        addr: *off as u16,
                        stride: 1 << s,
                    })
                } else {
                    Place::Ram(Loc::Rel { reg, off: disp })
                }
            }
            _ => Place::Ram(Loc::Rel { reg, off: disp }),
        }
    }

    fn load8(&mut self, id: usize, pc: u16, m: Mem, st: &St) -> Val {
        let a = self.address(m, st);
        match self.place(m, &a) {
            Place::Rom(x) => Val::k(u16::from(self.board.read(x))),
            Place::Table(at, by) => {
                self.ev(id, Ev::Table { pc, at, by });
                Val::of(K::Tab { at, by })
            }
            Place::Ram(loc) => {
                let val = match st.mem.get(&loc) {
                    Some(v) if !self.volatile.contains(&loc) => v.clone().with_from(Some(loc)),
                    _ => st.unwritten(loc),
                };
                let fresh = !st.wrote.contains(&loc);
                self.ev(
                    id,
                    Ev::Read {
                        pc,
                        loc,
                        val: val.clone(),
                        fresh,
                    },
                );
                val
            }
        }
    }

    fn store(&mut self, id: usize, pc: u16, loc: Loc, st: &mut St, v: Val) {
        if let Some((l, 0, d)) = v.lin()
            && l == loc
            && d != 0
            && (-16..16).contains(&d)
        {
            self.ev(
                id,
                Ev::Step {
                    pc,
                    loc,
                    by: d as i8,
                    guards: st.guards.clone(),
                },
            );
        } else {
            self.ev(
                id,
                Ev::Write {
                    pc,
                    loc,
                    val: v.clone(),
                    guards: st.guards.clone(),
                    added: self.store_from_sum
                        && matches!(
                            st.f,
                            Flags::Res {
                                op: FlagOp::Add,
                                ..
                            } | Flags::Cmp { .. }
                        ),
                },
            );
        }
        // Only work RAM is kept: the code reads it back. The screen's
        // bytes stay events.
        if loc.addr().is_none_or(|a| self.board.is_ram(a)) {
            st.set(loc, v);
        } else {
            st.forget(loc);
        }
    }

    fn store8(&mut self, id: usize, pc: u16, m: Mem, st: &mut St, v: Val) {
        let a = self.address(m, st);
        if let Place::Ram(loc) = self.place(m, &a) {
            self.store(id, pc, loc, st, v.byte());
        }
    }

    fn get(&mut self, id: usize, pc: u16, s: Src, st: &St) -> Val {
        match s {
            Src::R(r) => st.get8(r),
            Src::N(n) => Val::k(u16::from(n)),
            Src::M(m) => self.load8(id, pc, m, st),
        }
    }

    fn put(&mut self, id: usize, pc: u16, d: Dst, st: &mut St, v: Val) {
        match d {
            Dst::R(r) => st.set8(r, v),
            Dst::M(m) => self.store8(id, pc, m, st, v),
        }
    }

    /// The jump table's cases: each entry and the index value it is for.
    pub fn table_targets(&self, at: u16, by: Option<(Loc, u8)>) -> Vec<(u16, Option<(Loc, u8)>)> {
        let shift = by.map_or(1, |b| b.1);
        let mut out = Vec::new();
        let merged: Option<BTreeSet<u8>> = by.map(|(l, _)| {
            self.index_values
                .iter()
                .filter(|(k, _)| match (k, l) {
                    (Loc::Field { addr: a, stride: s }, Loc::Field { addr: b, stride: t }) => {
                        a == &b && *s > 0 && t % s == 0
                    }
                    _ => **k == l,
                })
                .flat_map(|(_, v)| v.iter().copied())
                .collect()
        });
        let known = merged.as_ref().filter(|v| !v.is_empty());
        // Values the index is written with bound the table: every value up
        // to the largest, as a value not written may still be one a
        // calculation makes.
        let values: Vec<u16> = match known {
            Some(vs) if !vs.is_empty() => {
                let max = vs.iter().copied().max().unwrap_or(0);
                (0..=u16::from(max)).collect()
            }
            _ => (0..TABLE_MAX).map(|k| (k * 2) >> shift.min(1)).collect(),
        };
        for v in values {
            let off = v << shift;
            if !off.is_multiple_of(2) {
                continue;
            }
            let e = at.wrapping_add(off);
            // The table ends where the code map has an instruction.
            if known.is_none() && off > 0 && self.starts.contains(&e) {
                break;
            }
            let t = self.board.word(e);
            // A table's routines are near it (Tapper's within a few
            // hundred bytes); a word far off is data past the table's end.
            let near = t.abs_diff(at) <= TABLE_REACH;
            if !looks_like_code(self.board, t) || !near || (t >= at && t < e.wrapping_add(2)) {
                if known.is_none() {
                    break;
                }
                continue;
            }
            out.push((t, by.map(|(l, _)| (l, v as u8))));
        }
        out
    }

    /// Where a jump through `v` goes.
    fn resolve(&self, v: &Val) -> Vec<(u16, Option<(Loc, u8)>)> {
        match &v.k {
            K::Lin { t: None, off } => vec![(*off as u16, None)],
            K::TabW { at, by } => self.table_targets(*at, *by),
            K::Lin {
                t: Some((Loc::Word(p), 0)),
                off: 0,
            } => self
                .hints
                .get(p)
                .map(|s| s.iter().map(|&t| (t, None)).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// A branch: the taken and the fallen-through state, or one of them
    /// when the flags decide it.
    fn branch(&mut self, id: usize, pc: u16, cc: Cc, st: &St) -> (Option<St>, Option<St>) {
        let test = st.f.clone();
        if cc != Cc::B {
            self.ev(
                id,
                Ev::Branch {
                    pc,
                    cc,
                    test: test.clone(),
                    guards: st.guards.clone(),
                },
            );
        }
        match decide(cc, &test, st) {
            Some(true) => return (Some(st.clone()), None),
            Some(false) => return (None, Some(st.clone())),
            None => {}
        }
        let with = |taken: bool| {
            let mut s = st.clone();
            s.guards.insert(Guard {
                pc,
                cc,
                taken,
                test: test.clone(),
            });
            carry_fix(&mut s, cc, taken);
            s
        };
        (Some(with(true)), Some(with(false)))
    }

    /// What the instruction at `pc` does to a path: the next pcs and their
    /// states. A return joins `exit`.
    fn step(&mut self, id: usize, pc: u16, mut st: St, exit: &mut Option<St>) -> Vec<(u16, St)> {
        let ins = self.insn(pc);
        let next = pc.wrapping_add(u16::from(ins.len));
        self.store_from_sum = matches!(ins.sem, Sem::Ld(Dst::M(_), Src::R(R8::A)));
        let ret = |st: St, exit: &mut Option<St>| {
            *exit = Some(match exit.take() {
                None => st,
                Some(e) => e.join(&st),
            });
        };
        match ins.sem {
            Sem::Nop | Sem::Di | Sem::Ei | Sem::Halt | Sem::Scf | Sem::Ccf => {}
            Sem::Im(m) => {
                self.im.insert(m);
            }
            Sem::Ld(Dst::R(R8::I), Src::R(R8::A)) => {
                if let Some(i) = st.a.k8() {
                    self.i_reg.insert(i);
                }
            }
            Sem::Ld(Dst::R(R8::A), Src::R(R8::R)) => {
                self.ev(
                    id,
                    Ev::Read {
                        pc,
                        loc: Loc::Refresh,
                        val: Val::entry(Loc::Refresh),
                        fresh: true,
                    },
                );
                st.a = Val::taint([Loc::Refresh].into()).with_from(Some(Loc::Refresh));
                st.f = Flags::Unknown;
            }
            Sem::Ld(d, s) => {
                let v = self.get(id, pc, s, &st);
                self.put(id, pc, d, &mut st, v);
            }
            Sem::LdImm16(r, n) => st.set16(r, Val::k(n)),
            Sem::LdMem16(r, n) => {
                let v = if self.board.is_rom(n) {
                    Val::k(self.board.word(n))
                } else {
                    let loc = Loc::Word(n);
                    let v = match st.mem.get(&loc) {
                        Some(v) => v.clone().with_from(Some(loc)),
                        None => {
                            let lo = st
                                .mem
                                .get(&Loc::At(n))
                                .cloned()
                                .unwrap_or_else(|| st.unwritten(Loc::At(n)));
                            let hi = st
                                .mem
                                .get(&Loc::At(n.wrapping_add(1)))
                                .cloned()
                                .unwrap_or_else(|| st.unwritten(Loc::At(n.wrapping_add(1))));
                            if st.general {
                                Val::taint([loc].into()).with_from(Some(loc))
                            } else {
                                Val::pair(hi, lo)
                            }
                        }
                    };
                    let fresh = !st.wrote.contains(&loc);
                    self.ev(
                        id,
                        Ev::Read {
                            pc,
                            loc,
                            val: v.clone(),
                            fresh,
                        },
                    );
                    v
                };
                st.set16(r, v);
            }
            Sem::StMem16(n, r) => {
                let v = st.get16(r);
                if let Some(c) = v.konst()
                    && self.board.is_rom(c)
                {
                    self.hints.entry(n).or_default().insert(c);
                }
                self.store(id, pc, Loc::Word(n), &mut st, v.clone());
                st.mem.insert(Loc::At(n), v.half(false));
                st.mem.insert(Loc::At(n.wrapping_add(1)), v.half(true));
                st.wrote.insert(Loc::At(n));
                st.wrote.insert(Loc::At(n.wrapping_add(1)));
            }
            Sem::LdSp(_) => st.stack.clear(),
            Sem::Alu(op, s) => self.alu(id, pc, op, s, &mut st),
            Sem::Step(d, by) => {
                let old = self.get(id, pc, d.src(), &st);
                let v = old.add(i32::from(by)).byte();
                st.f = Flags::Res {
                    v: v.clone(),
                    op: if by > 0 { FlagOp::Inc } else { FlagOp::Dec },
                    src: old.from,
                    mask: None,
                };
                self.put(id, pc, d, &mut st, v);
            }
            Sem::Step16(r, by) => {
                let v = st.get16(r).add(i32::from(by));
                st.set16(r, v);
            }
            Sem::Add16(d, s, carry) => {
                let v = match carry {
                    None => st.get16(d).plus(&st.get16(s)),
                    Some(_) => {
                        let mut t = st.get16(d).places();
                        t.extend(st.get16(s).places());
                        Val::taint(t)
                    }
                };
                st.set16(d, v);
                st.f = Flags::Unknown;
            }
            Sem::Rot { op, dst, copy, acc } => {
                let old = self.get(id, pc, dst.src(), &st);
                let v = match op {
                    Rot::Rlc | Rot::Sla | Rot::Rl => old.shl().byte(),
                    _ => Val::taint(old.places()),
                };
                st.f = if acc {
                    Flags::Unknown
                } else {
                    Flags::Res {
                        v: v.clone(),
                        op: FlagOp::Shift,
                        src: old.from,
                        mask: None,
                    }
                };
                self.put(id, pc, dst, &mut st, v.clone());
                if let Some(r) = copy {
                    st.set8(r, v);
                }
            }
            Sem::Bit(n, s) => {
                let v = self.get(id, pc, s, &st);
                st.f = Flags::Bit { n, src: v.from, v };
            }
            Sem::SetBit(n, d, on, copy) => {
                let old = self.get(id, pc, d.src(), &st);
                let v = match old.k8() {
                    Some(c) => Val::k(u16::from(if on { c | (1 << n) } else { c & !(1 << n) })),
                    None => Val::taint(old.places()),
                };
                self.put(id, pc, d, &mut st, v.clone());
                if let Some(r) = copy {
                    st.set8(r, v);
                }
            }
            Sem::ExDeHl => std::mem::swap(&mut st.de, &mut st.hl),
            Sem::ExAf => {
                std::mem::swap(&mut st.a, &mut st.alt.0);
                std::mem::swap(&mut st.f, &mut st.alt.1);
            }
            Sem::Exx => {
                std::mem::swap(&mut st.bc, &mut st.alt.2);
                std::mem::swap(&mut st.de, &mut st.alt.3);
                std::mem::swap(&mut st.hl, &mut st.alt.4);
            }
            Sem::ExSp(r) => {
                let v = st.get16(r);
                let top = st.stack.pop().unwrap_or_else(Val::top);
                st.stack.push(v);
                st.set16(r, top);
            }
            Sem::Push(r) => {
                let v = st.get16(r);
                st.stack.push(v);
            }
            Sem::Pop(r) => {
                let v = st.stack.pop().unwrap_or_else(Val::top);
                st.set16(r, v);
            }
            Sem::In(r, port) => {
                let p = port.or_else(|| st.bc.half(false).k8());
                self.ev(id, Ev::In { pc, port: p });
                let v = match p {
                    Some(p) => Val::entry(Loc::Port(p)),
                    None => Val::top(),
                };
                st.f = Flags::Unknown;
                if let Some(r) = r {
                    st.set8(r, v);
                }
            }
            Sem::Out(port, s) => {
                let v = self.get(id, pc, s, &st);
                let p = port.or_else(|| st.bc.half(false).k8());
                if let (Some(base), Some(p), Some(x)) = (self.board.ctc, p, v.k8())
                    && p == base
                    && x & 1 == 0
                {
                    self.ctc_vectors.insert(x);
                }
                self.ev(
                    id,
                    Ev::Out {
                        pc,
                        port: p,
                        val: v,
                    },
                );
            }
            Sem::Block(b) => {
                match b {
                    Block::Ld { up, .. } => {
                        let (dst, src, len) = (st.de.clone(), st.hl.clone(), st.bc.clone());
                        self.ev(
                            id,
                            Ev::Block {
                                pc,
                                dst: dst.clone(),
                                src: src.clone(),
                                len: len.clone(),
                            },
                        );
                        // What the copy wrote is not known any more.
                        let range = match (dst.konst(), len.konst()) {
                            (Some(d), Some(n)) if up => Some((d, d.wrapping_add(n))),
                            (Some(d), Some(n)) => Some((d.wrapping_sub(n), d.wrapping_add(1))),
                            _ => None,
                        };
                        let gone: Vec<Loc> = st
                            .mem
                            .keys()
                            .copied()
                            .filter(|l| match (l.addr(), range) {
                                (Some(a), Some((s, e))) => a >= s && a < e,
                                _ => range.is_none(),
                            })
                            .collect();
                        for l in gone {
                            st.forget(l);
                            st.mem.remove(&l);
                        }
                        let n = len.konst().map(i32::from);
                        let mv = |v: Val| match n {
                            Some(n) if up => v.add(n),
                            Some(n) => v.add(-n),
                            None => Val::taint(v.places()),
                        };
                        st.de = mv(dst);
                        st.hl = mv(src);
                        st.bc = Val::k(0);
                    }
                    Block::Cp { .. } | Block::Io => {
                        st.bc = Val::top();
                        st.hl = Val::taint(st.hl.places());
                    }
                }
                st.f = Flags::Unknown;
            }
            Sem::Daa => {
                self.ev(id, Ev::Decimal { pc });
                st.a = Val::taint(st.a.places());
                st.f = Flags::Res {
                    v: st.a.clone(),
                    op: FlagOp::Other,
                    src: None,
                    mask: None,
                };
            }
            Sem::Cpl | Sem::Neg => {
                st.a = Val::taint(st.a.places());
                st.f = Flags::Unknown;
            }
            Sem::Rxd => {
                st.a = Val::taint(st.a.places());
                let hl = st.hl.clone();
                if let Place::Ram(l) = self.place(Mem::Pair(R16::Hl), &hl) {
                    self.store(id, pc, l, &mut st, Val::top());
                }
                st.f = Flags::Unknown;
            }
            Sem::Jp(None, t) => return vec![(t, st)],
            Sem::Jp(Some(cc), t) => {
                let (tk, fl) = self.branch(id, pc, cc, &st);
                return tk
                    .map(|s| (t, s))
                    .into_iter()
                    .chain(fl.map(|s| (next, s)))
                    .collect();
            }
            Sem::Djnz(t) => {
                let b = st.bc.half(true).add(-1).byte();
                st.set8(R8::B, b.clone());
                st.f = Flags::B;
                return match b.k8() {
                    Some(0) => vec![(next, st)],
                    Some(_) => vec![(t, st)],
                    None => vec![(t, st.clone()), (next, st)],
                };
            }
            Sem::Call(None, t) | Sem::Rst(t) => return self.do_call(id, pc, t, next, st),
            Sem::Call(Some(cc), t) => {
                let (tk, fl) = self.branch(id, pc, cc, &st);
                let mut out = Vec::new();
                if let Some(s) = tk {
                    out.extend(self.do_call(id, pc, t, next, s));
                }
                out.extend(fl.map(|s| (next, s)));
                return out;
            }
            Sem::Ret(None) | Sem::Reti => return self.do_ret(id, pc, st, exit, &ret),
            Sem::Ret(Some(cc)) => {
                let (tk, fl) = self.branch(id, pc, cc, &st);
                let mut out = Vec::new();
                if let Some(s) = tk {
                    out.extend(self.do_ret(id, pc, s, exit, &ret));
                }
                out.extend(fl.map(|s| (next, s)));
                return out;
            }
            Sem::JpInd(r) => {
                let v = st.get16(r);
                return self.jump_through(id, pc, v, st);
            }
        }
        vec![(next, st)]
    }

    fn do_call(&mut self, id: usize, pc: u16, t: u16, next: u16, st: St) -> Vec<(u16, St)> {
        if !self.board.is_rom(t) {
            return Vec::new();
        }
        let (act, exit) = self.call(t, st.clone(), Some((id, pc, st.guards.clone())));
        self.ev(
            id,
            Ev::Call {
                pc,
                to: t,
                act,
                guards: st.guards.clone(),
            },
        );
        match exit {
            None => Vec::new(),
            Some(e) => {
                let mut n = if st.general || e.general {
                    over(&st, e)
                } else {
                    e
                };
                n.stack = st.stack.clone();
                n.guards = st.guards.clone();
                n.general = st.general;
                vec![(next, n)]
            }
        }
    }

    fn do_ret(
        &mut self,
        id: usize,
        pc: u16,
        mut st: St,
        exit: &mut Option<St>,
        ret: &dyn Fn(St, &mut Option<St>),
    ) -> Vec<(u16, St)> {
        match st.stack.pop() {
            // `push hl; ret`: a jump.
            Some(v) => self.jump_through(id, pc, v, st),
            None => {
                ret(st, exit);
                Vec::new()
            }
        }
    }

    fn jump_through(&mut self, id: usize, pc: u16, v: Val, st: St) -> Vec<(u16, St)> {
        let to = self.resolve(&v);
        self.ev(
            id,
            Ev::Indirect {
                pc,
                target: v.clone(),
                to: to.clone(),
            },
        );
        let mut out = Vec::new();
        for (t, case) in to {
            if !matches!(v.k, K::Lin { t: None, .. }) {
                self.tables.insert((pc, t));
            }
            let mut s = st.clone();
            if let Some((loc, k)) = case {
                s.guards.insert(Guard {
                    pc,
                    cc: Cc::Z,
                    taken: true,
                    test: Flags::Is { loc, k },
                });
            }
            out.push((t, s));
        }
        out
    }

    fn alu(&mut self, id: usize, pc: u16, op: Alu, s: Src, st: &mut St) {
        let a = st.a.clone();
        let x = self.get(id, pc, s, st);
        match op {
            Alu::Add => {
                // A plus the low byte of a pair: the low byte of a 16-bit
                // sum, its carry the high byte's.
                if let Src::R(r) = s
                    && let Some((p, false)) = r.pair()
                    && (a.konst().is_none() || st.get16(p).konst().is_none())
                {
                    let w = st.get16(p);
                    let sum = w.plus(&a);
                    st.a = sum.half(false);
                    st.f = Flags::Carry { sum, w };
                    return;
                }
                let v = if s == Src::R(R8::A) {
                    a.shl()
                } else {
                    a.plus(&x)
                }
                .byte();
                st.f = Flags::Res {
                    v: v.clone(),
                    op: FlagOp::Add,
                    src: None,
                    mask: None,
                };
                st.a = v;
            }
            Alu::Adc => {
                // `ld a,h; adc a,0` after a carry from the low byte: the
                // high byte of the sum.
                if let Flags::Carry { sum, w } = &st.f
                    && x.konst() == Some(0)
                    && a == w.half(true)
                {
                    st.a = sum.half(true);
                } else {
                    let mut t = a.places();
                    t.extend(x.places());
                    st.a = Val::taint(t);
                }
                st.f = Flags::Unknown;
            }
            Alu::Sub | Alu::Cp => {
                st.f = Flags::Cmp {
                    lhs: a.clone(),
                    rhs: x.clone(),
                };
                if op == Alu::Sub {
                    st.a = match x.konst() {
                        Some(c) => a.add(-i32::from(c)).byte(),
                        None if s == Src::R(R8::A) => Val::k(0),
                        None => {
                            let mut t = a.places();
                            t.extend(x.places());
                            Val::taint(t)
                        }
                    };
                }
            }
            Alu::Sbc => {
                let mut t = a.places();
                t.extend(x.places());
                st.a = Val::taint(t);
                st.f = Flags::Unknown;
            }
            Alu::And | Alu::Or | Alu::Xor => {
                let same = s == Src::R(R8::A);
                let v = match (a.k8(), x.k8()) {
                    _ if same && op == Alu::Xor => Val::k(0),
                    _ if same => a.clone(),
                    (Some(p), Some(q)) => Val::k(u16::from(match op {
                        Alu::And => p & q,
                        Alu::Or => p | q,
                        _ => p ^ q,
                    })),
                    (_, Some(0)) if op != Alu::And => a.clone(),
                    _ => {
                        let mut t = a.places();
                        t.extend(x.places());
                        Val::taint(t)
                    }
                };
                st.f = Flags::Res {
                    v: if same { a.clone() } else { v.clone() },
                    op: match op {
                        Alu::And => FlagOp::And,
                        Alu::Or => FlagOp::Or,
                        _ => FlagOp::Xor,
                    },
                    src: a.from,
                    mask: if op == Alu::And && !same {
                        x.k8()
                    } else {
                        None
                    },
                };
                st.a = v;
            }
        }
    }

    /// The constants each place is written with, over every activation:
    /// the next pass's table bounds.
    pub fn index_values_now(&self) -> BTreeMap<Loc, BTreeSet<u8>> {
        let mut out: BTreeMap<Loc, BTreeSet<u8>> = BTreeMap::new();
        for a in &self.acts {
            for e in &a.events {
                if let Ev::Write { loc, val, .. } = e
                    && let Some(k) = val.k8()
                {
                    out.entry(*loc).or_default().insert(k);
                }
            }
        }
        out
    }
}

/// Where an address points.
enum Place {
    Rom(u16),
    Table(u16, Option<(Loc, u8)>),
    Ram(Loc),
}

/// A general activation's exit over its caller's state: the registers it
/// leaves and the places it wrote, the rest as the caller had them.
fn over(caller: &St, e: St) -> St {
    let mut n = caller.clone();
    // A general read starts every register as unknown with no places: a
    // register that leaves so was most likely not touched, and keeps the
    // caller's value (IX and IY across a call to the random routine).
    let keep = |c: &Val, x: Val| if x == Val::top() { c.clone() } else { x };
    n.a = keep(&caller.a, e.a);
    n.f = e.f;
    n.bc = keep(&caller.bc, e.bc);
    n.de = keep(&caller.de, e.de);
    n.hl = keep(&caller.hl, e.hl);
    n.ix = keep(&caller.ix, e.ix);
    n.iy = keep(&caller.iy, e.iy);
    n.alt = e.alt;
    for (k, v) in e.mem {
        n.forget(k);
        // What a general read left unknown is the place's own symbol to
        // the caller: what it holds now.
        let v = if matches!(v.k, K::Top(_)) && !caller.general {
            Val::entry(k).with_from(None)
        } else {
            v
        };
        n.mem.insert(k, v);
        n.wrote.insert(k);
    }
    n
}

/// On a branch by the carry of a sum made by halves, the high byte of a
/// pair that still holds the old high byte is the sum's (no carry) or one
/// less (a carry, before `inc h`).
fn carry_fix(st: &mut St, cc: Cc, taken: bool) {
    let Flags::Carry { sum, w } = st.f.clone() else {
        return;
    };
    let carry = match cc {
        Cc::C => taken,
        Cc::Nc => !taken,
        _ => return,
    };
    let old_hi = w.half(true);
    for r in [R16::Bc, R16::De, R16::Hl, R16::Ix, R16::Iy] {
        let p = st.get16(r);
        if p.half(false) == sum.half(false) && p.half(true) == old_hi {
            let hi = if carry {
                Val::of(K::Hi(Box::new(sum.clone()), -1))
            } else {
                sum.half(true)
            };
            st.set16(r, Val::pair(hi, sum.half(false)));
        }
    }
}

/// Whether the flags decide a condition.
fn decide(cc: Cc, f: &Flags, st: &St) -> Option<bool> {
    let _ = st;
    let (z, c) = match f {
        Flags::Cmp { lhs, rhs } => {
            let (a, b) = (lhs.k8()?, rhs.k8()?);
            (Some(a == b), Some(a < b))
        }
        Flags::Res { v, .. } => (v.k8().map(|x| x == 0), None),
        Flags::Bit { n, v, .. } => (v.k8().map(|x| x & (1 << n) == 0), None),
        _ => (None, None),
    };
    match cc {
        Cc::Z => z,
        Cc::Nz => z.map(|x| !x),
        Cc::C => c,
        Cc::Nc => c.map(|x| !x),
        _ => None,
    }
}

/// Whether a table's word could be a routine's address: in ROM, not 0, and
/// not the bytes an erased or empty ROM holds.
pub fn looks_like_code(board: &Board, t: u16) -> bool {
    t != 0 && board.is_rom(t) && !matches!(board.bytes(t), [0xff, 0xff, ..] | [0, 0, 0, 0])
}
