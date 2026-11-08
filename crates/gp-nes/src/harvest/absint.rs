//! A static reader of the ROM's code: an abstract interpretation of each
//! routine from the vectors, with every call followed in its caller's
//! state, so a value one routine sets for another (an argument in a scratch
//! byte, a register) arrives with what it was.
//!
//! A value is a sum, `plus - minus + off`, of what two places held when the
//! frame began, or unknown with the places it came from. Places the frame
//! hasn't written read as what they held when it began. A routine that never
//! returns (the main loop) starts every pass with nothing known: its jump
//! back to its start is the next frame. Each path carries the branch
//! outcomes that led to it (its guards); where paths meet, only the guards
//! both share stay.
//!
//! What it reads is kept as events, per activation (one routine run in one
//! caller state): reads with the value read, writes with the value and the
//! guards, increments and decrements, branch tests, table reads, calls and
//! indirect jumps. The rules in [`super`] read the events.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::codemap::{Insn, Mn, Mode, RomCode, UNKNOWN_BANK, Va, va_add, va_cpu, va_tag};

/// An index register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub enum Reg {
    X,
    Y,
}

/// A place in memory: an address, or an array's base indexed by a
/// register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct Loc {
    pub addr: Va,
    pub idx: Option<Reg>,
}

impl Loc {
    pub fn at(addr: Va) -> Loc {
        Loc { addr, idx: None }
    }
    pub fn rom(&self) -> bool {
        self.addr >= 0x8000
    }
    pub fn ram(&self) -> bool {
        self.addr < 0x0800
    }
}

/// What a value is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// `plus - minus + off`, of what the places held when the frame began;
    /// a constant when both are `None` (then `off` is 0 to 255).
    Lin {
        plus: Option<Loc>,
        minus: Option<Loc>,
        off: i16,
    },
    /// Unknown, from these places.
    Top(BTreeSet<Loc>),
}

/// A value, and the place it was last loaded from unchanged.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Val {
    pub k: Kind,
    pub from: Option<Loc>,
    /// The ROM table it was read from unchanged (`lda base,x`). It is kept
    /// when the index is a known constant and `from` names one byte: a
    /// jump through it is still a jump table, since the walk may not yet
    /// have seen the code that writes the index's other values.
    pub table: Option<Tab>,
    /// A byte shifted left `n` times, plus `c` (`lda b; asl a; tay; iny`),
    /// as `(b, n, c)`: what a table's index is made from.
    pub shl: Option<(Loc, u8, i16)>,
}

/// A ROM table read at an index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tab {
    /// The table's base and the index register.
    pub at: Loc,
    /// The RAM byte the index was made from and how many times it was
    /// shifted left, when the reader knows.
    pub by: Option<(Loc, u8)>,
}

impl Val {
    pub fn k(c: u8) -> Val {
        Val {
            k: Kind::Lin {
                plus: None,
                minus: None,
                off: i16::from(c),
            },
            from: None,
            table: None,
            shl: None,
        }
    }
    pub fn top() -> Val {
        Val {
            k: Kind::Top(BTreeSet::new()),
            from: None,
            table: None,
            shl: None,
        }
    }
    pub fn taint(locs: BTreeSet<Loc>) -> Val {
        Val {
            k: Kind::Top(locs),
            from: None,
            table: None,
            shl: None,
        }
    }
    pub fn entry(l: Loc) -> Val {
        Val {
            k: Kind::Lin {
                plus: Some(l),
                minus: None,
                off: 0,
            },
            from: Some(l),
            table: None,
            shl: None,
        }
    }
    /// The constant, if it is one.
    pub fn konst(&self) -> Option<u8> {
        match self.k {
            Kind::Lin {
                plus: None,
                minus: None,
                off,
            } => Some(off as u8),
            _ => None,
        }
    }
    /// `(plus, minus, off)` when it is a sum of places.
    pub fn lin(&self) -> Option<(Option<Loc>, Option<Loc>, i16)> {
        match self.k {
            Kind::Lin { plus, minus, off } => Some((plus, minus, off)),
            _ => None,
        }
    }
    /// Whether the value is a sum naming the place.
    pub fn names(&self, l: Loc) -> bool {
        matches!(self.k, Kind::Lin { plus, minus, .. } if plus == Some(l) || minus == Some(l))
    }

    /// Every place the value depends on.
    pub fn places(&self) -> BTreeSet<Loc> {
        let mut s = match &self.k {
            Kind::Lin { plus, minus, .. } => plus.iter().chain(minus.iter()).copied().collect(),
            Kind::Top(t) => t.clone(),
        };
        s.extend(self.from);
        s
    }
    fn join(&self, o: &Val) -> Val {
        if self == o {
            return self.clone();
        }
        let from = if self.from == o.from { self.from } else { None };
        let table = if self.table == o.table {
            self.table
        } else {
            None
        };
        let shl = if self.shl == o.shl { self.shl } else { None };
        if self.k == o.k {
            return Val {
                k: self.k.clone(),
                from,
                table,
                shl,
            };
        }
        let mut t = self.places();
        t.extend(o.places());
        Val {
            k: Kind::Top(t),
            from,
            table,
            shl,
        }
    }
    fn with_from(mut self, f: Option<Loc>) -> Val {
        self.from = f;
        self
    }
    fn with_table(mut self, t: Option<Tab>) -> Val {
        self.table = t;
        self
    }
    /// The table a jump's pointer byte was read from: `table`, else the
    /// indexed place it was loaded from.
    pub fn table_or_from(&self) -> Option<Loc> {
        self.table.map(|t| t.at).or(self.from)
    }
    /// The RAM byte an index register's value is made from: the byte, how
    /// many times it was shifted left, and the constant added after.
    pub fn index_byte(&self) -> Option<(Loc, u8, i16)> {
        if self.shl.is_some() {
            return self.shl;
        }
        let (l, c) = match (self.from, &self.k) {
            (Some(l), _) => (l, 0),
            (
                None,
                Kind::Lin {
                    plus: Some(p),
                    minus: None,
                    off,
                },
            ) => (*p, *off),
            _ => return None,
        };
        (l.ram() && l.idx.is_none()).then_some((l, 0, c))
    }
    /// The value plus a constant, wrapping.
    fn add(&self, d: i16) -> Val {
        let mut r = self.add_value(d);
        r.shl = self
            .index_byte()
            .filter(|&(_, n, _)| n > 0)
            .map(|(l, n, c)| (l, n, c + d));
        r
    }
    fn add_value(&self, d: i16) -> Val {
        match self.k {
            Kind::Lin {
                plus: None,
                minus: None,
                off,
            } => Val::k((off + d).rem_euclid(256) as u8),
            Kind::Lin { plus, minus, off } => {
                let mut o = off + d;
                // A sum of places keeps its offset signed, -128 to 127.
                o = (o + 128).rem_euclid(256) - 128;
                Val {
                    k: Kind::Lin {
                        plus,
                        minus,
                        off: o,
                    },
                    from: None,
                    table: None,
                    shl: None,
                }
            }
            Kind::Top(ref t) => Val::taint(t.clone()),
        }
    }
}

/// What the flags of the last instruction that set them describe.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Flags {
    Unknown,
    /// `CMP`, `CPX`, `CPY`: Z and C from `lhs - rhs`, unsigned.
    Cmp {
        lhs: Val,
        rhs: Val,
    },
    /// Z and N from a result; for `AND #m`, the masked place and the mask.
    Res {
        v: Val,
        op: Mn,
        src: Option<Loc>,
        mask: Option<u8>,
    },
    /// Carry from a shift or an add.
    Carry {
        op: Mn,
        src: Val,
    },
}

/// One branch outcome a path took.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Guard {
    pub pc: Va,
    pub op: Mn,
    pub taken: bool,
    pub test: Flags,
}

/// What a path knows of the board's bank registers: the bank in each
/// switched region ([`crate::codemap::Board::region`]: $8000, then $A000 on
/// MMC3), MMC1's serial register (bits written, value so far) and MMC3's
/// bank select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Banks {
    pub bank: [Option<u8>; 2],
    /// Where a bank not known came from: a RAM byte as the frame began,
    /// and the ROM table read at it when the bank is the table's entry
    /// (a bus-conflict table: `ldx byte; lda table,x; sta table,x`).
    pub from: [Option<(Loc, Option<Va>)>; 2],
    /// MMC1's serial register: how many bits it holds (when known), and
    /// their value (when every bit is known).
    pub shift: Option<(u8, Option<u8>)>,
    pub select: Option<u8>,
}

impl Banks {
    /// At power-on: no bank known, MMC1's serial register empty.
    pub fn power_on() -> Banks {
        Banks {
            bank: [None, None],
            from: [None, None],
            shift: Some((0, Some(0))),
            select: None,
        }
    }

    fn join(&self, o: &Banks) -> Banks {
        let same = |a: Option<u8>, b: Option<u8>| if a == b { a } else { None };
        let from = |a: Option<(Loc, Option<Va>)>, b| if a == b { a } else { None };
        Banks {
            bank: [same(self.bank[0], o.bank[0]), same(self.bank[1], o.bank[1])],
            from: [from(self.from[0], o.from[0]), from(self.from[1], o.from[1])],
            shift: if self.shift == o.shift {
                self.shift
            } else {
                None
            },
            select: same(self.select, o.select),
        }
    }
}

/// The machine state of one path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct St {
    pub banks: Banks,
    pub a: Val,
    pub x: Val,
    pub y: Val,
    pub c: Val,
    pub flags: Flags,
    pub mem: BTreeMap<Loc, Val>,
    pub stack: Vec<Val>,
    pub guards: BTreeSet<Guard>,
    /// Places every path to here has written this frame.
    pub wrote: BTreeSet<Loc>,
    /// Reads of places this frame hasn't written read as unknown, not as
    /// what they held when the frame began (a routine read out of context).
    pub general: bool,
}

impl St {
    pub fn fresh() -> St {
        St {
            banks: Banks::power_on(),
            a: Val::top(),
            x: Val::top(),
            y: Val::top(),
            c: Val::top(),
            flags: Flags::Unknown,
            mem: BTreeMap::new(),
            stack: Vec::new(),
            guards: BTreeSet::new(),
            wrote: BTreeSet::new(),
            general: false,
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
        St {
            banks: self.banks.join(&o.banks),
            a: self.a.join(&o.a),
            x: self.x.join(&o.x),
            y: self.y.join(&o.y),
            c: self.c.join(&o.c),
            flags: if self.flags == o.flags {
                self.flags.clone()
            } else {
                Flags::Unknown
            },
            mem,
            stack,
            guards: self.guards.intersection(&o.guards).cloned().collect(),
            wrote: self.wrote.intersection(&o.wrote).copied().collect(),
            general: self.general || o.general,
        }
    }

    fn unwritten(&self, l: Loc) -> Val {
        if self.general {
            Val::taint([l].into()).with_from(Some(l))
        } else {
            Val::entry(l)
        }
    }

    /// Writes a place. A value the reader can't name, or one that names
    /// the place itself (`x + 1` stored to `x`), is kept as the place's
    /// own symbol: what it holds now. Every other value that names the
    /// place's old content is forgotten first, so a symbol always means
    /// the current content. Returns what the place holds.
    pub fn set(&mut self, loc: Loc, v: Val) -> Val {
        let keep = !matches!(v.k, Kind::Top(_)) && !v.names(loc);
        // A copy of a place keeps where it came from (a pointer's bytes
        // loaded from a table), until that place changes.
        let copied = v.from.filter(|f| *f != loc);
        // A ROM table doesn't change, so a copy keeps it whatever it holds.
        let table = v.table;
        self.forget(loc);
        let stored = if keep {
            v.with_from(copied)
        } else if self.general {
            Val::taint([loc].into()).with_from(copied).with_table(table)
        } else {
            Val::entry(loc).with_from(copied).with_table(table)
        };
        self.mem.insert(loc, stored.clone());
        self.wrote.insert(loc);
        stored
    }

    /// The place changes: values that name its old content don't any more.
    fn forget(&mut self, loc: Loc) {
        let fix = |v: &mut Val| {
            if v.names(loc) {
                *v = Val::taint(v.places());
            } else if v.from == Some(loc) {
                v.from = None;
            }
        };
        fix(&mut self.a);
        fix(&mut self.x);
        fix(&mut self.y);
        fix(&mut self.c);
        self.mem.values_mut().for_each(fix);
        self.stack.iter_mut().for_each(fix);
        match &mut self.flags {
            Flags::Cmp { lhs, rhs } => {
                fix(lhs);
                fix(rhs);
            }
            Flags::Res { v, .. } => fix(v),
            Flags::Carry { src, .. } => fix(src),
            Flags::Unknown => {}
        }
    }

    /// A register changed: what was indexed by it is somewhere else now.
    fn reindex(&mut self, r: Reg) {
        self.mem.retain(|l, _| l.idx != Some(r));
    }
}

/// What the reader saw one instruction do, in one activation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ev {
    Read {
        pc: Va,
        loc: Loc,
        val: Val,
        index: Option<Val>,
        /// No path to here wrote the place this frame: it holds what the
        /// last frame left.
        fresh: bool,
    },
    Write {
        pc: Va,
        loc: Loc,
        val: Val,
        index: Option<Val>,
        guards: BTreeSet<Guard>,
    },
    /// `INC` or `DEC` of a place: +1 or -1.
    Step {
        pc: Va,
        loc: Loc,
        by: i8,
        guards: BTreeSet<Guard>,
    },
    Branch {
        pc: Va,
        op: Mn,
        test: Flags,
        guards: BTreeSet<Guard>,
    },
    Call {
        pc: Va,
        to: Va,
        act: usize,
        guards: BTreeSet<Guard>,
    },
    /// `JMP (ptr)`, with what the pointer's bytes held; or `RTS` to an
    /// address the routine pushed.
    Indirect { pc: Va, lo: Val, hi: Val, rts: bool },
}

/// One routine run in one caller state.
#[derive(Debug, Clone)]
pub struct Act {
    pub start: Va,
    pub routine: usize,
    pub general: bool,
    pub events: BTreeSet<Ev>,
    pub callers: BTreeSet<(usize, Va)>,
    /// Places it writes, and whether it returns.
    pub writes: BTreeSet<Loc>,
    pub returns: bool,
}

/// Activations per routine before the reader stops telling callers apart.
const CONTEXTS: usize = 24;
/// Calls deep before the same.
const DEPTH: usize = 24;
/// Visits of one instruction in one activation before its state widens to
/// unknown.
const VISITS: usize = 12;
/// Paths kept apart at one instruction by the constants in X and Y.
const SPLITS: usize = 4;
/// What keeps paths apart at an instruction: X's and Y's constants.
type Sig = Option<(Option<u8>, Option<u8>)>;

pub struct Reader<'a> {
    pub code: &'a RomCode,
    pub acts: Vec<Act>,
    memo: HashMap<(Va, St), (usize, Option<St>)>,
    general: BTreeMap<Va, (usize, Option<St>)>,
    per_routine: BTreeMap<Va, usize>,
    stack: Vec<Va>,
    /// Routines that never return: a frame loop.
    pub loops: BTreeSet<Va>,
    starts: BTreeSet<Va>,
    /// Routines that never return and aren't a frame loop or a vector: a
    /// path that jumps into one goes on in it, as code of its own routine,
    /// with its guards ([`continuations`]).
    pub inline: BTreeSet<Va>,
    /// Loop heads: targets of a backward branch or jump.
    heads: BTreeSet<Va>,
    /// Calls and jumps into a switched region whose bank wasn't known
    /// there, as (instruction, CPU target): not followed.
    pub unresolved: BTreeSet<(Va, u16)>,
    /// Routines in a bank the reader went into, for the code map's next
    /// walk.
    pub banked: BTreeSet<Va>,
    /// The banks a frame loop's paths end a frame with.
    frame_end: Option<Banks>,
    /// The RAM the interrupt handlers write, and the RAM the frames write
    /// (from the pass before; `None` on the first). What neither writes
    /// holds, in every frame and interrupt, what the reset code left.
    pub interrupt_writes: Option<BTreeSet<Loc>>,
    pub frame_writes: Option<BTreeSet<Loc>>,
    /// The RAM addresses the code names, directly or as an array's base
    /// (from the pass before): an indexed write reaches no further than
    /// the next of them.
    pub named: BTreeSet<Va>,
    /// Those bytes, with the reset code's values, once the frame loop is
    /// entered.
    start_consts: BTreeMap<Loc, u8>,
    /// What the code before the frame loop left in RAM, as the loop is
    /// first entered.
    loop_entry: Option<BTreeMap<Loc, Val>>,
    /// The values each RAM byte can hold as a frame begins (from the pass
    /// before, [`Reader::frame_values_now`]).
    pub frame_values: BTreeMap<Va, BTreeSet<u8>>,
    /// What each RAM pointer can hold, from the last pass's writes
    /// ([`pointer_hints`]): a jump through one the path doesn't know goes
    /// to each.
    pub hints: BTreeMap<Va, BTreeSet<Va>>,
    /// Jumps through a pointer the hints resolved, as (site, target).
    pub hinted: BTreeSet<(Va, Va)>,
}

impl<'a> Reader<'a> {
    pub fn new(code: &'a RomCode) -> Self {
        let starts: BTreeSet<Va> = code.routines.addr.iter().copied().collect();
        let mut loops = BTreeSet::new();
        for i in 0..code.routines.len() {
            let a = code.routines.addr[i];
            let end = a + code.routines.size[i];
            let mut leaves = false;
            for (_, ins) in code.insns.range(a..) {
                if ins.pc >= end {
                    break;
                }
                match ins.mn {
                    Mn::Rts | Mn::Rti | Mn::Brk => leaves = true,
                    Mn::Jmp if ins.mode == Mode::Ind => leaves = true,
                    Mn::Jmp if starts.contains(&ins.arg) && ins.arg != a => leaves = true,
                    _ => {}
                }
            }
            let falls_out = starts.contains(&end) && {
                let last = code.insns.range(..end).next_back().map(|(_, i)| *i);
                last.is_some_and(|l| {
                    !matches!(l.mn, Mn::Jmp | Mn::Rts | Mn::Rti) && !pulls_after(code, &l)
                })
            };
            if !leaves && !falls_out {
                loops.insert(a);
            }
        }
        let inline = continuations(code, &starts, &mut loops);
        let heads = code
            .insns
            .values()
            .filter(|i| {
                (i.mode == Mode::Rel || (i.mn == Mn::Jmp && i.mode == Mode::Abs)) && i.arg <= i.pc
            })
            .map(|i| i.arg)
            .collect();
        Reader {
            code,
            acts: Vec::new(),
            memo: HashMap::new(),
            general: BTreeMap::new(),
            per_routine: BTreeMap::new(),
            stack: Vec::new(),
            loops,
            starts,
            inline,
            heads,
            unresolved: BTreeSet::new(),
            banked: BTreeSet::new(),
            frame_end: None,
            interrupt_writes: None,
            frame_writes: None,
            named: BTreeSet::new(),
            start_consts: BTreeMap::new(),
            loop_entry: None,
            frame_values: BTreeMap::new(),
            hints: BTreeMap::new(),
            hinted: BTreeSet::new(),
        }
    }

    /// Reads from each vector with nothing known.
    pub fn run_vectors(&mut self) -> Vec<(String, Va, usize)> {
        let mut out = Vec::new();
        for (name, v) in [("reset", 0xFFFC), ("nmi", 0xFFFA), ("irq", 0xFFFE)] {
            let a = u32::from(self.code.word(v));
            if a < 0x8000 || out.iter().any(|(_, b, _)| *b == a) {
                continue;
            }
            // An interrupt starts from what no frame and no interrupt
            // writes, as the reset code left it.
            let mut entry = St::fresh();
            if name != "reset" {
                for (l, k) in &self.start_consts {
                    entry.mem.insert(*l, Val::k(*k));
                }
            }
            let (act, _) = self.call(a, entry, None);
            out.push((name.to_owned(), a, act));
        }
        out
    }

    /// Reads each of `starts` (routine starts runs of the ROM entered) that
    /// no path read: from a fresh state that knows the bank the start is in
    /// and the RAM the reset code left, as an interrupt starts. Returns how
    /// many it read.
    pub fn run_entries(&mut self, starts: &[Va]) -> usize {
        let mut ran: BTreeSet<Va> = self.acts.iter().map(|a| a.start).collect();
        for a in &self.acts {
            for e in &a.events {
                let (Ev::Step { pc, .. }
                | Ev::Read { pc, .. }
                | Ev::Write { pc, .. }
                | Ev::Branch { pc, .. }
                | Ev::Call { pc, .. }
                | Ev::Indirect { pc, .. }) = e;
                ran.insert(*pc);
            }
        }
        let mut read = 0;
        for &s in starts {
            if ran.contains(&s) || self.code.routines.at(s).is_none() {
                continue;
            }
            let mut entry = St::fresh();
            let cpu = crate::codemap::va_cpu(s);
            let tag = crate::codemap::va_tag(s);
            if tag > 0 && tag != crate::codemap::UNKNOWN_BANK {
                let i = usize::from((0xA000..0xC000).contains(&cpu) && self.code.board.mapper == 4);
                entry.banks.bank[i] = Some((tag - 1) as u8);
            }
            for (l, k) in &self.start_consts {
                entry.mem.insert(*l, Val::k(*k));
            }
            self.call(s, entry, None);
            ran.insert(s);
            read += 1;
        }
        read
    }

    fn routine_of(&self, pc: Va) -> usize {
        self.code.routines.at(pc).unwrap_or(usize::MAX)
    }

    /// Runs the routine at `start` from `entry`; returns its activation and
    /// its state on return.
    pub fn call(
        &mut self,
        start: Va,
        mut entry: St,
        caller: Option<(usize, Va)>,
    ) -> (usize, Option<St>) {
        if self.loops.contains(&start) {
            // Each frame starts fresh but for the banks, which stay as set,
            // and the RAM no frame and no interrupt writes, which holds what
            // the code before the loop left.
            let banks = entry.banks;
            let consts = self.frame_consts(&entry.mem);
            if self.start_consts.is_empty() {
                self.start_consts = consts.clone();
            }
            if self.loop_entry.is_none() {
                self.loop_entry = Some(entry.mem.clone());
            }
            entry = St::fresh();
            entry.banks = banks;
            for (l, k) in consts {
                entry.mem.insert(l, Val::k(k));
            }
        }
        entry.flags = Flags::Unknown;
        let caller_guards = std::mem::take(&mut entry.guards);
        let caller_stack = std::mem::take(&mut entry.stack);
        let n = self.per_routine.get(&start).copied().unwrap_or(0);
        let recursive = self.stack.contains(&start);
        let generalize = recursive || n >= CONTEXTS || self.stack.len() >= DEPTH;
        let key = (start, entry.clone());
        let found = if generalize {
            self.general.get(&start).cloned()
        } else {
            self.memo.get(&key).cloned()
        };
        let (act, exit) = match found {
            Some(r) => r,
            None if generalize && recursive => {
                // A call back into a routine being read: nothing known after.
                let mut s = entry.clone();
                s.a = Val::top();
                s.x = Val::top();
                s.y = Val::top();
                s.c = Val::top();
                s.mem.clear();
                return (usize::MAX, Some(s));
            }
            None => {
                let e = if generalize {
                    let mut g = St::fresh();
                    g.general = true;
                    g
                } else {
                    entry.clone()
                };
                *self.per_routine.entry(start).or_insert(0) += 1;
                let id = self.acts.len();
                self.acts.push(Act {
                    start,
                    routine: self.routine_of(start),
                    general: generalize,
                    events: BTreeSet::new(),
                    callers: BTreeSet::new(),
                    writes: BTreeSet::new(),
                    returns: false,
                });
                self.stack.push(start);
                let looping = self.loops.contains(&start);
                self.frame_end = None;
                let mut exit = self.body(id, start, e.clone());
                // A frame that ends with other banks, or other constants,
                // than it began with: the next frames are read again from
                // what both agree on, until they agree.
                if looping
                    && let Some(end) = self.frame_end.take()
                    && end != e.banks
                {
                    let mut e2 = e;
                    e2.banks = e2.banks.join(&end);
                    exit = self.body(id, start, e2);
                }
                self.stack.pop();
                self.acts[id].returns = exit.is_some();
                let r = (id, exit);
                if generalize {
                    self.general.insert(start, r.clone());
                } else {
                    self.memo.insert(key, r.clone());
                }
                r
            }
        };
        if let Some(c) = caller
            && act != usize::MAX
        {
            self.acts[act].callers.insert(c);
        }
        let exit = exit.map(|mut s| {
            if act != usize::MAX && self.acts[act].general {
                // Read out of context: what it writes is unknown to the caller.
                let mut back = entry.clone();
                for l in &self.acts[act].writes {
                    back.mem
                        .insert(*l, s.mem.get(l).cloned().unwrap_or_else(Val::top));
                }
                back.a = s.a;
                back.x = s.x;
                back.y = s.y;
                back.c = s.c;
                back.flags = s.flags;
                s = back;
            }
            s.guards = caller_guards.clone();
            s.stack = caller_stack.clone();
            s
        });
        (act, exit)
    }

    fn body(&mut self, id: usize, start: Va, entry: St) -> Option<St> {
        let looping = self.loops.contains(&start);
        // States by instruction and, where paths hold different constants
        // in X or Y, by those constants: a path that picked an index stays
        // apart from one that picked another until they agree, so a table
        // read at that index reads one entry per path (`ldx #0; ...; inx;
        // lda speeds,x`). At most SPLITS such keys per instruction, and
        // none at a loop's head, so a counted loop meets as one state.
        let mut states: BTreeMap<(Va, Sig), St> = BTreeMap::new();
        let mut visits: BTreeMap<(Va, Sig), usize> = BTreeMap::new();
        let mut todo: BTreeSet<(Va, Sig)> = BTreeSet::new();
        states.insert((start, None), entry);
        todo.insert((start, None));
        let mut exit: Option<St> = None;
        while let Some(key) = todo.pop_first() {
            let pc = key.0;
            let st = states[&key].clone();
            let Some(ins) = self.code.insns.get(&pc).copied() else {
                continue;
            };
            let outs = self.step(id, &ins, st, &mut exit);
            for (to, s) in outs {
                if looping && to == start {
                    // The next frame.
                    self.frame_end = Some(match self.frame_end {
                        Some(b) => b.join(&s.banks),
                        None => s.banks,
                    });
                    continue;
                }
                // A routine that never returns, or a jump engine called from
                // here, goes on in this path; any other start is a tail call.
                let goes_on = self.inline.contains(&to)
                    || pulls_after(self.code, &ins)
                    || self.hinted.contains(&(pc, to));
                if to != start && self.starts.contains(&to) && to != pc && !goes_on {
                    // Falls or branches into another routine: a tail call.
                    let g = s.guards.clone();
                    let (act, e) = self.call(to, s, Some((id, pc)));
                    self.ev(
                        id,
                        Ev::Call {
                            pc,
                            to,
                            act,
                            guards: g,
                        },
                    );
                    if let Some(e) = e {
                        exit = Some(match exit {
                            Some(x) => x.join(&e),
                            None => e,
                        });
                    }
                    continue;
                }
                let sig = (s.x.konst(), s.y.konst());
                let split = !self.heads.contains(&to)
                    && (sig.0.is_some() || sig.1.is_some())
                    && (states.contains_key(&(to, Some(sig)))
                        || states
                            .range((to, Some((None, None)))..=(to, Some((Some(255), Some(255)))))
                            .count()
                            < SPLITS);
                let key = (to, split.then_some(sig));
                let v = visits.entry(key).or_insert(0);
                let merged = match states.get(&key) {
                    None => s,
                    Some(old) => {
                        let mut j = old.join(&s);
                        if *v >= VISITS {
                            j.mem.values_mut().for_each(|x| *x = x.join(&Val::top()));
                        }
                        if &j == old {
                            continue;
                        }
                        j
                    }
                };
                *v += 1;
                states.insert(key, merged);
                todo.insert(key);
            }
        }
        exit
    }

    /// A jump through a table: each entry is read as a tail call from
    /// here, and what it returns with is this routine's return.
    #[allow(clippy::too_many_arguments)]
    fn through_table(
        &mut self,
        id: usize,
        pc: Va,
        lo: &Val,
        hi: &Val,
        rts: bool,
        st: St,
        exit: &mut Option<St>,
    ) {
        for (t, v) in table_cases(self.code, lo, hi, rts) {
            let Some(t) = self.resolve(pc, t, &st) else {
                continue;
            };
            let mut s = st.clone();
            if let Some((l, k)) = v {
                s.guards.insert(index_guard(pc, l, k));
            }
            let g = s.guards.clone();
            let (act, e) = self.call(t, s, Some((id, pc)));
            self.ev(
                id,
                Ev::Call {
                    pc,
                    to: t,
                    act,
                    guards: g,
                },
            );
            if let Some(e) = e {
                *exit = Some(match exit.take() {
                    Some(x) => x.join(&e),
                    None => e,
                });
            }
        }
    }

    /// Where a call, jump or table entry at `pc` to `t` goes: into a
    /// switched region, the bank the path has mapped there; `None` when it
    /// isn't known (the target is recorded and not followed).
    fn resolve(&mut self, pc: Va, t: Va, st: &St) -> Option<Va> {
        let c = va_cpu(t);
        let Some((start, _)) = self.code.board.region(c) else {
            return Some(t);
        };
        if va_tag(t) != 0 {
            return Some(t);
        }
        let i = usize::from(start == 0xA000);
        match st.banks.bank[i] {
            Some(b) => {
                let v = ((u32::from(b) + 1) << 16) | u32::from(c);
                self.banked.insert(v);
                Some(v)
            }
            None => {
                self.unresolved.insert((pc, c));
                None
            }
        }
    }

    /// Where an array at `base` ends for what an indexed write forgets: 16
    /// bytes on, or the next address the code names if that comes first.
    fn array_end(&self, base: Va) -> Va {
        let cap = base.saturating_add(16);
        self.named
            .range(base + 1..)
            .next()
            .map_or(cap, |&n| n.min(cap))
    }

    /// [`Reader::resolve`] for a call: a bank not known that the path read
    /// from a RAM byte goes to each bank the byte's values give
    /// ([`Reader::frame_values`]), each with the condition "byte == value".
    fn resolve_all(&mut self, pc: Va, t: Va, st: &St) -> Vec<(Va, Option<(Loc, u8)>)> {
        let c = va_cpu(t);
        if let Some((start, _)) = self.code.board.region(c)
            && va_tag(t) == 0
        {
            let i = usize::from(start == 0xA000);
            if st.banks.bank[i].is_none()
                && let Some((l, table)) = st.banks.from[i]
                && let Some(ks) = self.frame_values.get(&l.addr).cloned()
            {
                let mut out = Vec::new();
                for k in ks {
                    let b = match table {
                        Some(base) => self.code.read(va_add(base, u16::from(k))),
                        None => k,
                    };
                    let v = ((u32::from(b) + 1) << 16) | u32::from(c);
                    self.banked.insert(v);
                    out.push((v, Some((l, k))));
                }
                return out;
            }
        }
        self.resolve(pc, t, st)
            .map(|t| (t, None))
            .into_iter()
            .collect()
    }

    /// The values each RAM byte can hold as a frame begins: what the code
    /// before the frame loop left in it, and every constant a frame or an
    /// interrupt writes to it (none when one of those writes isn't a
    /// constant, or the code before the loop left it unknown).
    pub fn frame_values_now(&self) -> BTreeMap<Va, BTreeSet<u8>> {
        let Some(entry) = &self.loop_entry else {
            return BTreeMap::new();
        };
        let vectors: Vec<Va> = [0xFFFA, 0xFFFE]
            .map(|v| u32::from(self.code.word(v)))
            .to_vec();
        let roots: Vec<usize> = (0..self.acts.len())
            .filter(|&i| {
                let a = &self.acts[i];
                self.loops.contains(&a.start)
                    || (vectors.contains(&a.start) && a.callers.is_empty())
            })
            .collect();
        let mut seen: BTreeSet<usize> = roots.iter().copied().collect();
        let mut todo = roots;
        let mut writes: Vec<(Loc, Option<u8>)> = Vec::new();
        while let Some(i) = todo.pop() {
            for e in &self.acts[i].events {
                match e {
                    Ev::Write { loc, val, .. } if loc.ram() => writes.push((*loc, val.konst())),
                    Ev::Step { loc, .. } if loc.ram() => writes.push((*loc, None)),
                    Ev::Call { act, .. } if *act != usize::MAX && seen.insert(*act) => {
                        todo.push(*act)
                    }
                    _ => {}
                }
            }
        }
        let end = |base: Va| {
            self.named
                .range(base + 1..)
                .next()
                .copied()
                .unwrap_or(base + 0x100)
                .min(base + 0x100)
        };
        let mut out = BTreeMap::new();
        for (l, v) in entry {
            if !l.ram() || l.idx.is_some() {
                continue;
            }
            let Some(k0) = v.konst() else {
                continue;
            };
            let mut ks = BTreeSet::from([k0]);
            let mut ok = true;
            for (w, k) in &writes {
                let hits =
                    *w == *l || (w.idx.is_some() && w.addr <= l.addr && l.addr < end(w.addr));
                if hits {
                    match k {
                        Some(k) => {
                            ks.insert(*k);
                        }
                        None => ok = false,
                    }
                }
            }
            if ok && ks.len() <= 8 {
                out.insert(l.addr, ks);
            }
        }
        out
    }

    /// A data address as the path sees it: in a switched region, at the bank
    /// mapped there (that of the instruction's own code when it is in the
    /// same region), or at [`UNKNOWN_BANK`].
    fn data_addr(&self, pc: Va, a: Va, st: &St) -> Va {
        let c = va_cpu(a);
        let Some((start, size)) = self.code.board.region(c) else {
            return a;
        };
        if va_tag(a) != 0 {
            return a;
        }
        let r = (start, size);
        if va_tag(pc) != 0 && self.code.board.region(va_cpu(pc)) == Some(r) {
            return (pc & 0xFFFF_0000) | u32::from(c);
        }
        match st.banks.bank[usize::from(start == 0xA000)] {
            Some(b) => ((u32::from(b) + 1) << 16) | u32::from(c),
            None => (UNKNOWN_BANK << 16) | u32::from(c),
        }
    }

    /// A write to ROM on a board with banks: the mapper's registers. UxROM
    /// (2) takes the bank; MMC1 (1) shifts in bit 0 and, on the fifth
    /// write to $E000 to $FFFF, takes the bank (PRG mode 3, the last bank
    /// fixed); MMC3 (4) takes the select at even $8000 to $9FFE and banks
    /// 6 and 7 at odd addresses (mode 0). A value not known makes the
    /// register unknown. Whether it was a register write.
    fn bank_write(&self, st: &mut St, addr: u16, v: &Val) -> bool {
        let board = &self.code.board;
        if !crate::codemap::banked(&board.prg) {
            return false;
        }
        let k = v.konst();
        // A value read from a RAM byte this path hasn't written: the
        // byte as the frame began, directly or as a ROM table's entry.
        let src = if k.is_some() {
            None
        } else {
            let fresh = |l: Loc| l.ram() && l.idx.is_none() && !st.mem.contains_key(&l);
            match (v.table, v.from, &v.k) {
                (
                    Some(Tab {
                        at,
                        by: Some((l, 0)),
                    }),
                    _,
                    _,
                ) if fresh(l) => Some((l, Some(at.addr))),
                (None, Some(l), _) if fresh(l) => Some((l, None)),
                (
                    None,
                    None,
                    Kind::Lin {
                        plus: Some(l),
                        minus: None,
                        off: 0,
                    },
                ) if fresh(*l) => Some((*l, None)),
                _ => None,
            }
        };
        let b = &mut st.banks;
        match board.mapper {
            2 => {
                b.bank[0] = k;
                b.from[0] = src;
            }
            1 => match (k, b.shift) {
                (Some(k), _) if k & 0x80 != 0 => b.shift = Some((0, Some(0))),
                (_, Some((n, val))) => {
                    // The count goes on whether or not the bit is known.
                    let val = match (val, k) {
                        (Some(val), Some(k)) => Some(val | ((k & 1) << n)),
                        _ => None,
                    };
                    if n + 1 == 5 {
                        if addr >= 0xE000 {
                            b.bank[0] = val.map(|v| v & 0x0F);
                            b.from[0] = None;
                        }
                        b.shift = Some((0, Some(0)));
                    } else {
                        b.shift = Some((n + 1, val));
                    }
                }
                _ => {
                    // A count not known: the bank register may take
                    // anything.
                    if addr >= 0xE000 {
                        b.bank[0] = None;
                        b.from[0] = None;
                    }
                }
            },
            4 => {
                if (0x8000..0xA000).contains(&addr) {
                    if addr & 1 == 0 {
                        b.select = k;
                    } else {
                        match b.select {
                            Some(s) if s & 0x40 == 0 && s & 7 == 6 => {
                                b.bank[0] = k;
                                b.from[0] = src;
                            }
                            Some(s) if s & 0x40 == 0 && s & 7 == 7 => {
                                b.bank[1] = k;
                                b.from[1] = src;
                            }
                            Some(s) if s & 7 < 6 => {}
                            _ => b.bank = [None, None],
                        }
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// The RAM bytes a frame can start with: those holding a constant in
    /// `mem` that no interrupt and no frame writes (none, before those
    /// writes are known).
    fn frame_consts(&self, mem: &BTreeMap<Loc, Val>) -> BTreeMap<Loc, u8> {
        let (Some(wi), Some(wf)) = (&self.interrupt_writes, &self.frame_writes) else {
            return BTreeMap::new();
        };
        // An array runs from its base to the next address the code names.
        let end = |base: Va| {
            self.named
                .range(base + 1..)
                .next()
                .copied()
                .unwrap_or(base + 0x100)
                .min(base + 0x100)
        };
        let hit = |l: &Loc| {
            wi.iter()
                .chain(wf)
                .any(|x| *x == *l || (x.idx.is_some() && x.addr <= l.addr && l.addr < end(x.addr)))
        };
        mem.iter()
            .filter(|(l, _)| l.ram() && l.idx.is_none() && !hit(l))
            .filter_map(|(l, v)| v.konst().map(|k| (*l, k)))
            .collect()
    }

    /// The RAM the interrupt handlers write: every activation the NMI and
    /// IRQ vectors' reads lead to by calls.
    pub fn interrupt_writes_now(&self) -> BTreeSet<Loc> {
        let vectors: Vec<Va> = [0xFFFA, 0xFFFE]
            .map(|v| u32::from(self.code.word(v)))
            .to_vec();
        self.writes_from(|a| vectors.contains(&a.start) && a.callers.is_empty())
    }

    /// The RAM addresses the code names: every place a read or write uses,
    /// directly or as an array's base.
    pub fn named_now(&self) -> BTreeSet<Va> {
        self.acts
            .iter()
            .flat_map(|a| a.events.iter())
            .filter_map(|e| match e {
                Ev::Read { loc, .. } | Ev::Write { loc, .. } | Ev::Step { loc, .. } => {
                    loc.ram().then_some(loc.addr)
                }
                _ => None,
            })
            .collect()
    }

    /// The RAM the frames write: every activation a frame loop's reads lead
    /// to by calls.
    pub fn frame_writes_now(&self) -> BTreeSet<Loc> {
        self.writes_from(|a| self.loops.contains(&a.start))
    }

    fn writes_from(&self, root: impl Fn(&Act) -> bool) -> BTreeSet<Loc> {
        let mut todo: Vec<usize> = (0..self.acts.len())
            .filter(|&i| root(&self.acts[i]))
            .collect();
        let mut seen: BTreeSet<usize> = todo.iter().copied().collect();
        let mut out = BTreeSet::new();
        while let Some(i) = todo.pop() {
            out.extend(self.acts[i].writes.iter().filter(|l| l.ram()).copied());
            for e in &self.acts[i].events {
                if let Ev::Call { act, .. } = e
                    && *act != usize::MAX
                    && seen.insert(*act)
                {
                    todo.push(*act);
                }
            }
        }
        out
    }

    fn ev(&mut self, id: usize, e: Ev) {
        if let Ev::Write { loc, .. } | Ev::Step { loc, .. } = &e {
            self.acts[id].writes.insert(*loc);
        }
        self.acts[id].events.insert(e);
    }

    /// The place an instruction's operand names, and the index's value.
    fn place(&self, ins: &Insn, st: &St) -> Option<(Loc, Option<Val>)> {
        let (addr, idx) = match ins.mode {
            Mode::Zp | Mode::Abs => (ins.arg, None),
            Mode::ZpX | Mode::AbsX => (ins.arg, Some(Reg::X)),
            Mode::ZpY | Mode::AbsY | Mode::IndY => (ins.arg, Some(Reg::Y)),
            Mode::IndX => (ins.arg, Some(Reg::X)),
            _ => return None,
        };
        let addr = if matches!(ins.mode, Mode::Abs | Mode::AbsX | Mode::AbsY) {
            self.data_addr(ins.pc, addr, st)
        } else {
            addr
        };
        let iv = idx.map(|r| {
            if r == Reg::X {
                st.x.clone()
            } else {
                st.y.clone()
            }
        });
        // A ROM table at a known index is one byte.
        if addr >= 0x8000
            && let Some(k) = iv.as_ref().and_then(Val::konst)
            && !matches!(ins.mode, Mode::IndY | Mode::IndX)
        {
            return Some((Loc::at(va_add(addr, u16::from(k))), None));
        }
        Some((Loc { addr, idx }, iv))
    }

    fn load(&mut self, id: usize, ins: &Insn, st: &St) -> (Val, Option<Loc>) {
        if ins.mode == Mode::Imm {
            return (Val::k(ins.arg as u8), None);
        }
        let Some((loc, iv)) = self.place(ins, st) else {
            return (Val::top(), None);
        };
        // A ROM table read at an index: `lda base,x`, or `lda (ptr),y` with
        // the pointer known to point into ROM. An index made as `b << n`
        // plus `c` puts the table at `base + c`, indexed by `b << n`.
        let rom_ptr = || {
            let get = |a: Va| st.mem.get(&Loc::at(a)).and_then(Val::konst);
            let p = u16::from_le_bytes([get(ins.arg)?, get(va_add(ins.arg, 1))?]);
            (p >= 0x8000).then(|| self.data_addr(ins.pc, u32::from(p), st))
        };
        let table = match ins.mode {
            Mode::AbsX | Mode::ZpX if ins.arg >= 0x8000 => {
                Some((self.data_addr(ins.pc, ins.arg, st), Reg::X, &st.x))
            }
            Mode::AbsY | Mode::ZpY if ins.arg >= 0x8000 => {
                Some((self.data_addr(ins.pc, ins.arg, st), Reg::Y, &st.y))
            }
            Mode::IndY => rom_ptr().map(|p| (p, Reg::Y, &st.y)),
            _ => None,
        }
        .map(|(base, r, iv)| {
            let by = iv.index_byte();
            let c = by.map_or(0, |(_, _, c)| c);
            Tab {
                at: Loc {
                    addr: va_add(base, c as u16),
                    idx: Some(r),
                },
                by: by.map(|(l, n, _)| (l, n)),
            }
        });
        let v = if va_tag(loc.addr) == UNKNOWN_BANK {
            Val::taint([loc].into()).with_from(Some(loc))
        } else if matches!(ins.mode, Mode::IndY | Mode::IndX) {
            Val::taint([loc].into())
                .with_from(Some(loc))
                .with_table(table)
        } else if let Some(v) = st.mem.get(&loc) {
            v.clone().with_from(Some(loc))
        } else if loc.rom() && loc.idx.is_none() {
            Val::k(self.code.read(loc.addr))
                .with_from(Some(loc))
                .with_table(table)
        } else if loc.rom() || (0x2000..0x4020).contains(&loc.addr) {
            Val::taint([loc].into())
                .with_from(Some(loc))
                .with_table(table)
        } else {
            st.unwritten(loc)
        };
        self.ev(
            id,
            Ev::Read {
                pc: ins.pc,
                loc,
                val: v.clone(),
                index: iv,
                fresh: !st.wrote.contains(&loc) && loc.ram(),
            },
        );
        (v, Some(loc))
    }

    fn store(&mut self, id: usize, ins: &Insn, st: &mut St, v: Val) {
        let Some((loc, iv)) = self.place(ins, st) else {
            return;
        };
        self.ev(
            id,
            Ev::Write {
                pc: ins.pc,
                loc,
                val: v.clone(),
                index: iv,
                guards: st.guards.clone(),
            },
        );
        // A register reads back something else than was written.
        if matches!(ins.mode, Mode::IndY | Mode::IndX) || (0x2000..0x4020).contains(&loc.addr) {
            return;
        }
        if va_cpu(loc.addr) >= 0x8000 && self.bank_write(st, va_cpu(loc.addr), &v) {
            return;
        }
        // An indexed write may land anywhere in its array: up to 16 bytes
        // on, and no further than the next address the code names.
        if loc.idx.is_some() {
            let base = loc.addr;
            let end = self.array_end(base);
            st.mem
                .retain(|l, _| !(l.idx.is_none() && l.addr >= base && l.addr < end));
        } else {
            let ends: Vec<(Va, Va)> = st
                .mem
                .keys()
                .filter(|l| l.idx.is_some())
                .map(|l| (l.addr, self.array_end(l.addr)))
                .collect();
            st.mem.retain(|l, _| {
                !(l.idx.is_some()
                    && ends
                        .iter()
                        .any(|&(b, e)| b == l.addr && b <= loc.addr && loc.addr < e))
            });
        }
        let stored = st.set(loc, v);
        let held = stored.with_from(Some(loc));
        match ins.mn {
            Mn::Sta => st.a = held,
            Mn::Stx => st.x = held,
            Mn::Sty => st.y = held,
            _ => {}
        }
    }

    fn step(&mut self, id: usize, ins: &Insn, mut st: St, exit: &mut Option<St>) -> Vec<(Va, St)> {
        let next = ins.next();
        let res = |v: &Val, op: Mn, src: Option<Loc>| Flags::Res {
            v: v.clone(),
            op,
            src,
            mask: None,
        };
        match ins.mn {
            Mn::Lda | Mn::Ldx | Mn::Ldy => {
                let (v, src) = self.load(id, ins, &st);
                st.flags = res(&v, ins.mn, src);
                match ins.mn {
                    Mn::Lda => st.a = v,
                    Mn::Ldx => {
                        st.x = v;
                        st.reindex(Reg::X);
                    }
                    _ => {
                        st.y = v;
                        st.reindex(Reg::Y);
                    }
                }
            }
            Mn::Sta | Mn::Stx | Mn::Sty => {
                let v = match ins.mn {
                    Mn::Sta => st.a.clone(),
                    Mn::Stx => st.x.clone(),
                    _ => st.y.clone(),
                };
                self.store(id, ins, &mut st, v);
            }
            Mn::Tax | Mn::Tay | Mn::Txa | Mn::Tya | Mn::Tsx | Mn::Txs => match ins.mn {
                Mn::Tax => {
                    st.x = st.a.clone();
                    st.reindex(Reg::X);
                    st.flags = res(&st.x, ins.mn, st.x.from);
                }
                Mn::Tay => {
                    st.y = st.a.clone();
                    st.reindex(Reg::Y);
                    st.flags = res(&st.y, ins.mn, st.y.from);
                }
                Mn::Txa => {
                    st.a = st.x.clone();
                    st.flags = res(&st.a, ins.mn, st.a.from);
                }
                Mn::Tya => {
                    st.a = st.y.clone();
                    st.flags = res(&st.a, ins.mn, st.a.from);
                }
                Mn::Tsx => {
                    st.x = Val::top();
                    st.reindex(Reg::X);
                }
                _ => {}
            },
            Mn::Inx | Mn::Dex | Mn::Iny | Mn::Dey => {
                let d = if matches!(ins.mn, Mn::Inx | Mn::Iny) {
                    1
                } else {
                    -1
                };
                let (r, reg) = if matches!(ins.mn, Mn::Inx | Mn::Dex) {
                    (&mut st.x, Reg::X)
                } else {
                    (&mut st.y, Reg::Y)
                };
                let from = r.from;
                *r = r.add(d);
                let v = r.clone();
                st.reindex(reg);
                st.flags = res(&v, ins.mn, from);
            }
            Mn::Inc | Mn::Dec => {
                let (v, src) = self.load(id, ins, &st);
                let by: i8 = if ins.mn == Mn::Inc { 1 } else { -1 };
                let nv = v.add(i16::from(by));
                if let Some((loc, _)) = self.place(ins, &st) {
                    self.ev(
                        id,
                        Ev::Step {
                            pc: ins.pc,
                            loc,
                            by,
                            guards: st.guards.clone(),
                        },
                    );
                    let stored = st.set(loc, nv);
                    st.flags = res(&stored, ins.mn, src);
                } else {
                    st.flags = res(&nv, ins.mn, src);
                }
            }
            Mn::Clc => st.c = Val::k(0),
            Mn::Sec => st.c = Val::k(1),
            Mn::Adc | Mn::Sbc => {
                let (b, _) = self.load(id, ins, &st);
                let a = st.a.clone();
                let sub = ins.mn == Mn::Sbc;
                let carry = st.c.konst();
                let out = match (a.lin(), b.lin(), carry) {
                    // A constant plus a sum of places, adding.
                    (Some((None, None, k)), Some((Some(_), _, _)), Some(c)) if !sub => {
                        st.c = Val::taint(a.places().union(&b.places()).copied().collect());
                        b.add(k + i16::from(c))
                    }
                    (Some(_), Some((None, None, k)), Some(c)) => {
                        let d = if sub {
                            -(k) - 1 + i16::from(c)
                        } else {
                            k + i16::from(c)
                        };
                        let r = a.add(d);
                        // The carry out is known when both are constants.
                        st.c = match (a.konst(), b.konst()) {
                            (Some(x), Some(y)) => {
                                let (x, y, c) = (i16::from(x), i16::from(y), i16::from(c));
                                if sub {
                                    Val::k(u8::from(x - y - 1 + c >= 0))
                                } else {
                                    Val::k(u8::from(x + y + c > 255))
                                }
                            }
                            _ => Val::taint(a.places().union(&b.places()).copied().collect()),
                        };
                        r
                    }
                    (Some((Some(p), None, o)), Some((Some(q), None, o2)), Some(c)) if sub => {
                        st.c = Val::taint([p, q].into());
                        Val {
                            k: Kind::Lin {
                                plus: Some(p),
                                minus: Some(q),
                                off: o - o2 - 1 + i16::from(c),
                            },
                            from: None,
                            table: None,
                            shl: None,
                        }
                        .add(0)
                    }
                    _ => {
                        let mut t = a.places();
                        t.extend(b.places());
                        t.extend(st.c.places());
                        st.c = Val::taint(t.clone());
                        Val::taint(t)
                    }
                };
                st.flags = res(&out, ins.mn, None);
                st.a = out;
            }
            Mn::And | Mn::Ora | Mn::Eor => {
                let (b, _) = self.load(id, ins, &st);
                let a = st.a.clone();
                let out = match (a.konst(), b.konst()) {
                    (Some(x), Some(y)) => Val::k(match ins.mn {
                        Mn::And => x & y,
                        Mn::Ora => x | y,
                        _ => x ^ y,
                    }),
                    _ => {
                        let mut t = a.places();
                        t.extend(b.places());
                        Val::taint(t)
                    }
                };
                st.flags = Flags::Res {
                    v: out.clone(),
                    op: ins.mn,
                    src: a.from,
                    mask: if ins.mn == Mn::And { b.konst() } else { None },
                };
                st.a = out;
            }
            Mn::Cmp | Mn::Cpx | Mn::Cpy => {
                let (b, _) = self.load(id, ins, &st);
                let lhs = match ins.mn {
                    Mn::Cmp => st.a.clone(),
                    Mn::Cpx => st.x.clone(),
                    _ => st.y.clone(),
                };
                st.flags = Flags::Cmp { lhs, rhs: b };
                st.c = Val::top();
            }
            Mn::Bit => {
                let (v, src) = self.load(id, ins, &st);
                st.flags = res(&v, ins.mn, src);
            }
            Mn::Asl | Mn::Lsr | Mn::Rol | Mn::Ror => {
                let (v, src, place) = if ins.mode == Mode::Acc {
                    (st.a.clone(), st.a.from, None)
                } else {
                    let (v, s) = self.load(id, ins, &st);
                    (v, s, self.place(ins, &st))
                };
                let c_in = st.c.clone();
                let out = match (v.konst(), c_in.konst()) {
                    (Some(x), _) if ins.mn == Mn::Asl => {
                        st.c = Val::k(x >> 7);
                        Val::k(x << 1)
                    }
                    (Some(x), _) if ins.mn == Mn::Lsr => {
                        st.c = Val::k(x & 1);
                        Val::k(x >> 1)
                    }
                    (Some(x), Some(c)) if ins.mn == Mn::Rol => {
                        st.c = Val::k(x >> 7);
                        Val::k((x << 1) | c)
                    }
                    (Some(x), Some(c)) => {
                        st.c = Val::k(x & 1);
                        Val::k((x >> 1) | (c << 7))
                    }
                    _ => {
                        let mut t = v.places();
                        if matches!(ins.mn, Mn::Rol | Mn::Ror) {
                            t.extend(c_in.places());
                        }
                        st.c = Val::taint(v.places());
                        Val::taint(t)
                    }
                };
                st.flags = Flags::Carry {
                    op: ins.mn,
                    src: v.clone().with_from(src),
                };
                if ins.mode == Mode::Acc {
                    let mut out = out;
                    if ins.mn == Mn::Asl {
                        out.shl = v
                            .index_byte()
                            .map(|(l, n, c)| (l, n.saturating_add(1), c * 2));
                    }
                    st.a = out;
                } else if let Some((loc, _)) = place {
                    self.ev(
                        id,
                        Ev::Write {
                            pc: ins.pc,
                            loc,
                            val: out.clone(),
                            index: None,
                            guards: st.guards.clone(),
                        },
                    );
                    st.set(loc, out);
                }
            }
            Mn::Pha | Mn::Php => {
                let v = if ins.mn == Mn::Pha {
                    st.a.clone()
                } else {
                    Val::top()
                };
                st.stack.push(v);
            }
            Mn::Pla | Mn::Plp => {
                let v = st.stack.pop().unwrap_or_else(Val::top);
                if ins.mn == Mn::Pla {
                    st.flags = res(&v, ins.mn, v.from);
                    st.a = v;
                } else {
                    st.flags = Flags::Unknown;
                    st.c = Val::top();
                }
            }
            Mn::Clv | Mn::Cld | Mn::Cli | Mn::Sed | Mn::Sei | Mn::Nop => {}
            Mn::Brk => return Vec::new(),
            Mn::Rts | Mn::Rti => {
                if ins.mn == Mn::Rts && st.stack.len() >= 2 {
                    // An address the routine pushed: a jump through a table.
                    let lo = st.stack[st.stack.len() - 1].clone();
                    let hi = st.stack[st.stack.len() - 2].clone();
                    self.ev(
                        id,
                        Ev::Indirect {
                            pc: ins.pc,
                            lo: lo.clone(),
                            hi: hi.clone(),
                            rts: true,
                        },
                    );
                    let mut s2 = st.clone();
                    s2.stack.truncate(s2.stack.len() - 2);
                    self.through_table(id, ins.pc, &lo, &hi, true, s2, exit);
                    return Vec::new();
                }
                *exit = Some(match exit.take() {
                    Some(x) => x.join(&st),
                    None => st,
                });
                return Vec::new();
            }
            Mn::Jsr
                if {
                    let c = va_cpu(ins.arg);
                    self.code.board.region(c).is_some_and(|(start, _)| {
                        let i = usize::from(start == 0xA000);
                        va_tag(ins.arg) == 0
                            && st.banks.bank[i].is_none()
                            && st.banks.from[i].is_some()
                    })
                } =>
            {
                // A bank a RAM byte chose: each bank its values give.
                let targets = self.resolve_all(ins.pc, ins.arg, &st);
                if targets.is_empty() {
                    let mut s = st;
                    s.a = Val::top();
                    s.x = Val::top();
                    s.y = Val::top();
                    s.c = Val::top();
                    s.flags = Flags::Unknown;
                    s.mem.clear();
                    return vec![(next, s)];
                }
                let mut joined: Option<St> = None;
                for (to, g) in targets {
                    let mut s = st.clone();
                    if let Some((l, k)) = g {
                        s.guards.insert(index_guard(ins.pc, l, k));
                    }
                    let gs = s.guards.clone();
                    let (act, e) = self.call(to, s, Some((id, ins.pc)));
                    self.ev(
                        id,
                        Ev::Call {
                            pc: ins.pc,
                            to,
                            act,
                            guards: gs,
                        },
                    );
                    if let Some(mut e) = e {
                        e.guards = st.guards.clone();
                        joined = Some(match joined {
                            Some(j) => j.join(&e),
                            None => e,
                        });
                    }
                }
                return joined.map(|s| vec![(next, s)]).unwrap_or_default();
            }
            Mn::Jsr => {
                let Some(to) = self.resolve(ins.pc, ins.arg, &st) else {
                    // Into a bank not known here: what it does isn't read.
                    let mut s = st;
                    s.a = Val::top();
                    s.x = Val::top();
                    s.y = Val::top();
                    s.c = Val::top();
                    s.flags = Flags::Unknown;
                    s.mem.clear();
                    return vec![(next, s)];
                };
                if pulls_after(self.code, ins) {
                    // A routine that pulls its return address (a jump
                    // engine) goes on in this path: the address of the
                    // call's last byte is on the stack, high byte first.
                    let mut s = st;
                    let [lo, hi] = va_cpu(ins.pc).wrapping_add(2).to_le_bytes();
                    s.stack.push(Val::k(hi));
                    s.stack.push(Val::k(lo));
                    return vec![(to, s)];
                }
                let g = st.guards.clone();
                let (act, e) = self.call(to, st, Some((id, ins.pc)));
                self.ev(
                    id,
                    Ev::Call {
                        pc: ins.pc,
                        to,
                        act,
                        guards: g,
                    },
                );
                return e.map(|s| vec![(next, s)]).unwrap_or_default();
            }
            Mn::Jmp if ins.mode == Mode::Ind => {
                let p = ins.arg;
                let get = |a: Va| {
                    st.mem
                        .get(&Loc::at(a))
                        .cloned()
                        .unwrap_or_else(|| st.unwritten(Loc::at(a)))
                };
                // The CPU reads the high byte from the same page: a pointer
                // at $xxFF takes it from $xx00.
                let (lo, hi) = (get(p), get(ptr_hi(p)));
                self.ev(
                    id,
                    Ev::Indirect {
                        pc: ins.pc,
                        lo: lo.clone(),
                        hi: hi.clone(),
                        rts: false,
                    },
                );
                let out = table_branches(self.code, ins.pc, &lo, &hi, &st);
                if !out.is_empty() {
                    return out
                        .into_iter()
                        .filter_map(|(t, s)| self.resolve(ins.pc, t, &s).map(|t| (t, s)))
                        .collect();
                }
                // A pointer whose bytes the path knows: a jump there.
                if let (Some(l), Some(h)) = (lo.konst(), hi.konst()) {
                    let t = u32::from(u16::from_le_bytes([l, h]));
                    if va_cpu(t) < 0x8000 {
                        return Vec::new();
                    }
                    return match self.resolve(ins.pc, t, &st) {
                        Some(t) => {
                            self.hinted.insert((ins.pc, t));
                            vec![(t, st)]
                        }
                        None => Vec::new(),
                    };
                }
                // A pointer this path didn't set: what the program ever
                // writes to it.
                let names = |v: &Val, l: Va| v.from == Some(Loc::at(l)) || v.names(Loc::at(l));
                if names(&lo, p)
                    && names(&hi, ptr_hi(p))
                    && let Some(ts) = self.hints.get(&p).cloned()
                {
                    let mut out = Vec::new();
                    for t in ts {
                        if let Some(t) = self.resolve(ins.pc, t, &st) {
                            self.hinted.insert((ins.pc, t));
                            out.push((t, st.clone()));
                        }
                    }
                    return out;
                }
                return Vec::new();
            }
            Mn::Jmp => {
                return self
                    .resolve(ins.pc, ins.arg, &st)
                    .map(|to| vec![(to, st)])
                    .unwrap_or_default();
            }
            Mn::Bcc | Mn::Bcs | Mn::Beq | Mn::Bne | Mn::Bmi | Mn::Bpl | Mn::Bvc | Mn::Bvs => {
                let test = st.flags.clone();
                self.ev(
                    id,
                    Ev::Branch {
                        pc: ins.pc,
                        op: ins.mn,
                        test: test.clone(),
                        guards: st.guards.clone(),
                    },
                );
                let decided = decide(ins.mn, &test, &st);
                let mut out = Vec::new();
                for taken in [true, false] {
                    if decided.is_some_and(|d| d != taken) {
                        continue;
                    }
                    let mut s = st.clone();
                    let g = Guard {
                        pc: ins.pc,
                        op: ins.mn,
                        taken,
                        test: test.clone(),
                    };
                    // A test of a value that depends on no place says nothing
                    // about the path.
                    let empty = super::cond::of(&g).is_some_and(
                        |c| matches!(&c.subj, super::cond::Subj::Other(v) if v.is_empty()),
                    );
                    if !empty {
                        s.guards.insert(g);
                    }
                    out.push((if taken { ins.arg } else { next }, s));
                }
                return out;
            }
        }
        vec![(next, st)]
    }
}

/// What each RAM pointer (two bytes, low then high) can hold, from every
/// write the reader saw: when each write to either byte, and each indexed
/// write that can reach one, is a constant or a byte read from a ROM table
/// at an index, the pointer holds one of the constant pairs a routine
/// writes (in order) or one of the tables' entries. A pointer with any
/// other write gets no hint.
pub fn pointer_hints(code: &RomCode, acts: &[Act]) -> BTreeMap<Va, BTreeSet<Va>> {
    // Every write to RAM by place: its activation, pc and value.
    let mut writes: BTreeMap<Va, Vec<(usize, Va, Val)>> = BTreeMap::new();
    let mut indexed: Vec<(Va, Val)> = Vec::new();
    for (i, a) in acts.iter().enumerate() {
        for e in &a.events {
            if let Ev::Write { pc, loc, val, .. } = e
                && loc.ram()
            {
                match loc.idx {
                    None => writes
                        .entry(loc.addr)
                        .or_default()
                        .push((i, *pc, val.clone())),
                    Some(_) => indexed.push((loc.addr, val.clone())),
                }
            }
        }
    }
    let known = |v: &Val| v.konst().is_some() || v.table.is_some();
    let mut out = BTreeMap::new();
    for (&p, lo_w) in &writes {
        let q = ptr_hi(p);
        let Some(hi_w) = writes.get(&q) else {
            continue;
        };
        if !lo_w.iter().chain(hi_w).all(|w| known(&w.2))
            || indexed
                .iter()
                .any(|(base, v)| *base <= q && p.saturating_sub(*base) < 0x100 && !known(v))
        {
            continue;
        }
        let mut ts: BTreeSet<Va> = BTreeSet::new();
        let acts_of: BTreeSet<usize> = lo_w.iter().map(|w| w.0).collect();
        for i in acts_of {
            let mut l: Vec<&(usize, Va, Val)> = lo_w.iter().filter(|w| w.0 == i).collect();
            let mut h: Vec<&(usize, Va, Val)> = hi_w.iter().filter(|w| w.0 == i).collect();
            l.sort_by_key(|w| w.1);
            h.sort_by_key(|w| w.1);
            for (a, b) in l.iter().zip(&h) {
                match (a.2.konst(), b.2.konst()) {
                    (Some(x), Some(y)) => {
                        ts.insert(u32::from(u16::from_le_bytes([x, y])));
                    }
                    _ => {
                        for (t, _) in table_cases(code, &a.2, &b.2, false) {
                            ts.insert(t);
                        }
                    }
                }
            }
        }
        ts.retain(|&t| va_cpu(t) >= 0x8000);
        if !ts.is_empty() && ts.len() <= 64 {
            out.insert(p, ts);
        }
    }
    out
}

/// Where `JMP (p)` reads its target's high byte: the next byte, in the same
/// page (the 6502 doesn't carry into the high byte of the address).
fn ptr_hi(p: Va) -> Va {
    (p & 0xFFFF_FF00) | ((p + 1) & 0xFF)
}

/// Whether an instruction is a call to a routine that pulls its return
/// address.
fn pulls_after(code: &RomCode, ins: &Insn) -> bool {
    ins.mn == Mn::Jsr && crate::codemap::pulls_return(&|a| code.read(a), ins.arg)
}

/// The routines a frame loop goes on into, which never return: no path
/// from one reaches a return, following its jumps into other routines, its
/// jump tables' entries, a fall into the next routine and calls to a jump
/// engine. A path that goes into one goes on in it. It also adds to
/// `loops` the frame loops whose frame goes through such routines: where a
/// cycle of them that never returns is entered from outside it (from the
/// reset code, or a call).
fn continuations(code: &RomCode, starts: &BTreeSet<Va>, loops: &mut BTreeSet<Va>) -> BTreeSet<Va> {
    let mut next: BTreeMap<Va, Vec<Va>> = BTreeMap::new();
    let mut returns: BTreeSet<Va> = BTreeSet::new();
    let mut called: BTreeSet<Va> = BTreeSet::new();
    for i in 0..code.routines.len() {
        let a = code.routines.addr[i];
        let end = a + code.routines.size[i];
        let mut succ = Vec::new();
        let mut last = None;
        for (_, ins) in code.insns.range(a..) {
            if ins.pc >= end {
                break;
            }
            last = Some(*ins);
            match ins.mn {
                Mn::Rts | Mn::Rti | Mn::Brk => {
                    returns.insert(a);
                }
                Mn::Jmp if ins.mode == Mode::Ind => match code.jumps.get(&ins.pc) {
                    Some(es) => succ.extend(es.iter().copied().filter(|e| starts.contains(e))),
                    None => {
                        returns.insert(a);
                    }
                },
                Mn::Jmp if starts.contains(&ins.arg) && ins.arg != a => succ.push(ins.arg),
                Mn::Jsr if pulls_after(code, ins) && starts.contains(&ins.arg) => {
                    succ.push(ins.arg);
                }
                Mn::Jsr => {
                    called.insert(ins.arg);
                }
                _ => {}
            }
        }
        if starts.contains(&end)
            && last.is_some_and(|l| {
                !matches!(l.mn, Mn::Jmp | Mn::Rts | Mn::Rti) && !pulls_after(code, &l)
            })
        {
            succ.push(end);
        }
        next.insert(a, succ);
    }
    // The greatest set closed under "every successor is in it or a loop".
    let mut never: BTreeSet<Va> = next
        .keys()
        .copied()
        .filter(|a| !returns.contains(a))
        .collect();
    loop {
        let drop: Vec<Va> = never
            .iter()
            .copied()
            .filter(|a| {
                next[a]
                    .iter()
                    .any(|t| !never.contains(t) && !loops.contains(t))
            })
            .collect();
        if drop.is_empty() {
            break;
        }
        for d in drop {
            never.remove(&d);
        }
    }
    let reach = |from: Va| -> BTreeSet<Va> {
        let mut seen = BTreeSet::new();
        let mut todo = vec![from];
        while let Some(a) = todo.pop() {
            for &t in next.get(&a).into_iter().flatten() {
                if never.contains(&t) && seen.insert(t) {
                    todo.push(t);
                }
            }
        }
        seen
    };
    let vectors = [0xFFFA, 0xFFFC, 0xFFFE].map(|v| u32::from(code.word(v)));
    // A cycle's head: on a cycle, and entered from outside it.
    let reached: BTreeMap<Va, BTreeSet<Va>> = never.iter().map(|&a| (a, reach(a))).collect();
    for (&a, ra) in &reached {
        if !ra.contains(&a) {
            continue;
        }
        let outside = vectors.contains(&a)
            || called.contains(&a)
            || next
                .iter()
                .any(|(q, succ)| succ.contains(&a) && !ra.contains(q) && *q != a);
        if outside {
            loops.insert(a);
        }
    }
    // Only what a frame loop goes on into: the rest of the frame, read as
    // the loop's own code. Other code that never returns (a test's last
    // routine) keeps its own reading.
    let mut out: BTreeSet<Va> = BTreeSet::new();
    let mut todo: Vec<Va> = loops.iter().copied().collect();
    while let Some(a) = todo.pop() {
        for &t in next.get(&a).into_iter().flatten() {
            if never.contains(&t) && !loops.contains(&t) && !vectors.contains(&t) && out.insert(t) {
                todo.push(t);
            }
        }
    }
    out
}

/// A branch whose test is on constants goes one way.
fn decide(op: Mn, f: &Flags, st: &St) -> Option<bool> {
    match f {
        Flags::Cmp { lhs, rhs } => {
            let (a, b) = (lhs.konst()?, rhs.konst()?);
            Some(match op {
                Mn::Beq => a == b,
                Mn::Bne => a != b,
                Mn::Bcs => a >= b,
                Mn::Bcc => a < b,
                Mn::Bmi => a.wrapping_sub(b) & 0x80 != 0,
                Mn::Bpl => a.wrapping_sub(b) & 0x80 == 0,
                _ => return None,
            })
        }
        Flags::Res { v, .. } => {
            let a = v.konst()?;
            Some(match op {
                Mn::Beq => a == 0,
                Mn::Bne => a != 0,
                Mn::Bmi => a & 0x80 != 0,
                Mn::Bpl => a & 0x80 == 0,
                _ => return None,
            })
        }
        Flags::Carry { .. } | Flags::Unknown => match op {
            Mn::Bcc => st.c.konst().map(|c| c == 0),
            Mn::Bcs => st.c.konst().map(|c| c == 1),
            _ => None,
        },
    }
}

/// An indirect jump through a table read at an index is a branch to each
/// entry. The path to an entry holds the index byte's value for it (entry
/// `i` is at offset `i`, or `2i` in a table of words, and the index is the
/// byte shifted left), as a compare chain's would. The path goes on in this
/// routine or, at another routine's start, is a tail call.
fn table_branches(code: &RomCode, pc: Va, lo: &Val, hi: &Val, st: &St) -> Vec<(Va, St)> {
    // Once the rules have read a site's table (and bounded it), its
    // entries are those.
    let known = code.jumps.get(&pc);
    table_cases(code, lo, hi, false)
        .into_iter()
        .filter(|(t, _)| known.is_none_or(|es| es.contains(t)))
        .map(|(t, v)| {
            let mut s = st.clone();
            if let Some((l, k)) = v {
                s.guards.insert(index_guard(pc, l, k));
            }
            (t, s)
        })
        .collect()
}

/// The condition a jump table's entry holds: the index byte `l` is `k`.
fn index_guard(pc: Va, l: Loc, k: u8) -> Guard {
    let mut lhs = Val::top();
    lhs.from = Some(l);
    Guard {
        pc,
        op: Mn::Beq,
        taken: true,
        test: Flags::Cmp {
            lhs,
            rhs: Val::k(k),
        },
    }
}

/// The entries of an indirect jump's table, each with the index byte's
/// value that reaches it when the reader knows which byte the index is
/// made from: entry `i` is at offset `i`, or `2i` in a table of words, and
/// the index is the byte shifted left `n` times, so the value is the
/// offset shifted right `n` times.
pub fn table_cases(code: &RomCode, lo: &Val, hi: &Val, rts: bool) -> Vec<(Va, Option<(Loc, u8)>)> {
    let (Some(a), Some(b)) = (lo.table_or_from(), hi.table_or_from()) else {
        return Vec::new();
    };
    let split = b.addr != a.addr.wrapping_add(1);
    let by = lo
        .table
        .and_then(|t| t.by)
        .filter(|_| lo.table.map(|t| t.by) == hi.table.map(|t| t.by));
    table_entries(code, a, b, rts)
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let off = if split { i } else { 2 * i };
            let v = by.and_then(|(l, n)| {
                (n < 8 && off % (1usize << n) == 0 && (off >> n) < 256)
                    .then(|| (l, (off >> n) as u8))
            });
            (t, v)
        })
        .collect()
}

/// The entries of a jump table whose low and high bytes are read from `lo`
/// and `hi` at one index: two tables side by side (`lo` then `hi`, as many
/// entries as lie between them) or one table of words (`hi` is `lo + 1`).
/// For an `RTS` jump each entry is one less than its target. An entry ends
/// the table when its target isn't in ROM, doesn't start with code, or the
/// table runs into code the walk read.
pub fn table_entries(code: &RomCode, lo: Loc, hi: Loc, rts: bool) -> Vec<Va> {
    if !(lo.rom() && hi.rom() && lo.idx.is_some() && lo.idx == hi.idx) {
        return Vec::new();
    }
    let split = hi.addr != va_add(lo.addr, 1);
    let n = if split {
        (i64::from(hi.addr) - i64::from(lo.addr))
            .unsigned_abs()
            .min(128) as u16
    } else {
        128
    };
    let mut out = Vec::new();
    for i in 0..n {
        let (l, h) = if split {
            (va_add(lo.addr, i), va_add(hi.addr, i))
        } else {
            (va_add(lo.addr, 2 * i), va_add(lo.addr, 2 * i + 1))
        };
        let t = u16::from_le_bytes([code.read(l), code.read(h)]).wrapping_add(u16::from(rts));
        let t = code.board.target(lo.addr, t);
        if va_cpu(t) < 0x8000
            || !looks_like_code(code, t)
            || code.insns.contains_key(&l)
            || code.insns.contains_key(&h)
        {
            break;
        }
        out.push(t);
    }
    out
}

/// Whether an address starts code: up to four official instructions, none
/// a BRK, before a return or a jump.
pub fn looks_like_code(code: &RomCode, mut pc: Va) -> bool {
    for _ in 0..4 {
        let Some((mn, mode)) = crate::codemap::decode(code.read(pc)) else {
            return false;
        };
        if mn == Mn::Brk {
            return false;
        }
        if matches!(mn, Mn::Rts | Mn::Rti | Mn::Jmp) {
            return true;
        }
        pc = va_add(pc, mode.len());
    }
    true
}
