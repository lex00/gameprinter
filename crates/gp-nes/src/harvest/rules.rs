//! The rules that turn the reader's events into model parts. Each rule is
//! a pattern NES games share (a frame counter, a controller read, a state
//! byte, a slot array, a tile probe, a 16-bit speed), not a fact about one
//! game. A rule that can't decide leaves an open fact naming the routine and
//! bytes that would.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::absint::{Ev, Guard, Kind, Loc, Reader, Reg, Val};
use super::cond::{self, Cond, Rel, Subj};
use super::{
    Constant, Dispatch, Element, Evidence, Fact, Harvest, Input, Machine, MapRow, Open, RoutineRow,
    State, Transition,
};
use crate::codemap::{Insn, Mn, Mode, RomCode, Va};

const BUTTONS: [&str; 8] = ["a", "b", "select", "start", "up", "down", "left", "right"];

/// The places of a value's kind, without where it was loaded from.
fn kind_places(v: &Val) -> BTreeSet<Loc> {
    match &v.k {
        Kind::Lin { plus, minus, .. } => plus.iter().chain(minus.iter()).copied().collect(),
        Kind::Top(t) => t.clone(),
    }
}

fn hx(l: &Loc) -> String {
    let a = if l.addr > 0xFFFF {
        crate::codemap::va_hex(l.addr)
    } else {
        format!("{:02X}", l.addr)
    };
    match l.idx {
        None => format!("${a}"),
        Some(Reg::X) => format!("${a},x"),
        Some(Reg::Y) => format!("${a},y"),
    }
}

/// A 16-bit add or subtract the code does in one run of instructions:
/// `(hi:lo) = (hi:lo) +- (add_hi:add_lo)`.
#[derive(Debug, Clone)]
struct Chain {
    pc: Va,
    sub: bool,
    lo: Loc,
    hi: Loc,
    /// The low and high operands: a place or a constant.
    add_lo: Result<Loc, u8>,
    add_hi: Result<Loc, u8>,
    /// A third byte the carry goes on into.
    top: Option<Loc>,
}

/// A loop: its head, its back branch, the register it counts with and
/// how many times.
#[derive(Debug, Clone)]
struct Loop {
    head: Va,
    back: Va,
    reg: Option<Reg>,
    count: Option<u16>,
    /// `DEC m; BNE head`: counted down from what `m` held.
    by_byte: Option<Loc>,
    /// The instruction a skip goes to (the count step).
    next: Va,
}

pub(super) struct H<'a> {
    pub code: &'a RomCode,
    pub r: &'a Reader<'a>,
    pub inp: &'a Input,
    chains: BTreeMap<usize, Vec<BTreeSet<Guard>>>,
    /// Each call's guards by (caller activation, call site, callee
    /// activation), in the caller's event order.
    calls: std::collections::HashMap<(usize, Va, usize), Vec<&'a BTreeSet<Guard>>>,
    pub out: Harvest,
    cited: BTreeSet<Va>,
    /// Each jump table's site and entries, as `(site, entry)`.
    pub tables: Vec<(Va, Va)>,
}

impl<'a> H<'a> {
    pub fn new(code: &'a RomCode, r: &'a Reader<'a>, inp: &'a Input) -> Self {
        let mut h = H {
            code,
            r,
            inp,
            chains: BTreeMap::new(),
            calls: std::collections::HashMap::new(),
            out: Harvest::default(),
            cited: BTreeSet::new(),
            tables: Vec::new(),
        };
        for (c, a) in r.acts.iter().enumerate() {
            for e in &a.events {
                if let Ev::Call {
                    pc, act, guards, ..
                } = e
                {
                    h.calls.entry((c, *pc, *act)).or_default().push(guards);
                }
            }
        }
        for i in 0..r.acts.len() {
            h.chain_of(i);
        }
        h
    }

    fn chain_of(&mut self, act: usize) -> Vec<BTreeSet<Guard>> {
        if let Some(c) = self.chains.get(&act) {
            return c.clone();
        }
        self.chains.insert(act, vec![BTreeSet::new()]);
        let callers: Vec<(usize, Va)> = self.r.acts[act].callers.iter().copied().collect();
        let mut out: BTreeSet<BTreeSet<Guard>> = BTreeSet::new();
        if callers.is_empty() {
            out.insert(BTreeSet::new());
        }
        // At most 48 chains: a routine called along more paths keeps the
        // first 48, in the callers' order.
        'callers: for (c, pc) in callers {
            let gs: Vec<&BTreeSet<Guard>> =
                self.calls.get(&(c, pc, act)).cloned().unwrap_or_default();
            for up in self.chain_of(c) {
                for g in &gs {
                    let mut s = up.clone();
                    s.extend(g.iter().cloned());
                    out.insert(s);
                    if out.len() >= 48 {
                        break 'callers;
                    }
                }
            }
        }
        let v: Vec<_> = out.into_iter().collect();
        self.chains.insert(act, v.clone());
        v
    }

    /// The conditions on each path to an event: the caller chains with the
    /// event's own guards.
    fn paths(&self, act: usize, own: &BTreeSet<Guard>) -> Vec<Vec<Cond>> {
        let mut out = Vec::new();
        for ch in self.chains.get(&act).cloned().unwrap_or_default() {
            let mut cs: Vec<Cond> = ch
                .iter()
                .chain(own.iter())
                .filter_map(cond::of)
                .filter(|c| {
                    !c.places()
                        .iter()
                        .any(|l| (0x2000..0x4020).contains(&l.addr))
                })
                .collect();
            cs.sort();
            cs.dedup();
            out.push(cs);
        }
        out.sort();
        out.dedup();
        out
    }

    fn events(&self) -> impl Iterator<Item = (usize, &'a Ev)> + 'a {
        let r: &'a Reader<'a> = self.r;
        r.acts
            .iter()
            .enumerate()
            .flat_map(|(i, a)| a.events.iter().map(move |e| (i, e)))
    }

    pub fn rname(&self, pc: Va) -> String {
        match self.code.routines.at(pc) {
            Some(i) => self.code.routines.names[i].clone(),
            None => crate::codemap::routine_name(pc),
        }
    }

    fn ev_at(&mut self, pc: Va) -> Evidence {
        self.cited.insert(pc);
        Evidence {
            routine: self.rname(pc),
            pc: crate::codemap::va_hex(pc),
            insn: self.code.insns.get(&pc).map(Insn::text).unwrap_or_default(),
        }
    }

    fn fact(&mut self, path: impl Into<String>, value: Value, rule: &str, pcs: &[Va]) {
        let mut pcs = pcs.to_vec();
        pcs.sort_unstable();
        pcs.dedup();
        let evidence = pcs.iter().map(|&p| self.ev_at(p)).collect();
        let path = path.into();
        if self.out.facts.iter().any(|f| f.path == path) {
            return;
        }
        self.out.facts.push(Fact {
            path,
            value,
            rule: rule.to_owned(),
            evidence,
        });
    }

    fn open(
        &mut self,
        what: impl Into<String>,
        pc: Option<Va>,
        bytes: &[Loc],
        why: impl Into<String>,
    ) {
        let what = what.into();
        if self.out.open.iter().any(|o| o.what == what) {
            return;
        }
        self.out.open.push(Open {
            what,
            routine: pc.map(|p| self.rname(p)).unwrap_or_default(),
            pc: pc.map(crate::codemap::va_hex).unwrap_or_default(),
            bytes: bytes.iter().map(hx).collect(),
            why: why.into(),
        });
    }

    fn table_of(&self, addr: Va) -> Option<&super::Table> {
        if addr < 0x8000 || self.inp.prg_len == 0 {
            return None;
        }
        let off = if crate::codemap::banked(&self.code.board.prg) {
            self.code.board.prg_offset(addr)?
        } else {
            (addr as usize - 0x8000) % self.inp.prg_len
        };
        self.inp
            .tables
            .iter()
            .find(|t| off >= t.offset && off < t.offset + t.len.max(1))
    }

    fn all_insns(&self) -> Vec<Insn> {
        self.code.insns.values().copied().collect()
    }

    /// Every 16-bit add and subtract in the code.
    fn chains16(&self) -> Vec<Chain> {
        let v = self.all_insns();
        let mem = |i: &Insn| matches!(i.mode, Mode::Zp | Mode::Abs).then(|| Loc::at(i.arg));
        let opnd = |i: &Insn| {
            if i.mode == Mode::Imm {
                Some(Err(i.arg as u8))
            } else {
                mem(i).map(Ok)
            }
        };
        let mut out = Vec::new();
        for w in 0..v.len() {
            // CLC/SEC within the first three, then LDA a, ADC b, STA a,
            // LDA c, ADC d, STA c.
            let s = &v[w..v.len().min(w + 9)];
            let Some(ci) = s
                .iter()
                .take(3)
                .position(|i| matches!(i.mn, Mn::Clc | Mn::Sec))
            else {
                continue;
            };
            let rest: Vec<&Insn> = s
                .iter()
                .enumerate()
                .filter(|(k, _)| *k != ci)
                .map(|(_, i)| i)
                .collect();
            if rest.len() < 6 || ci > 1 {
                continue;
            }
            let sub = s[ci].mn == Mn::Sec;
            let op = if sub { Mn::Sbc } else { Mn::Adc };
            let (l1, a1, s1, l2, a2, s2) = (rest[0], rest[1], rest[2], rest[3], rest[4], rest[5]);
            if l1.mn != Mn::Lda
                || a1.mn != op
                || s1.mn != Mn::Sta
                || l2.mn != Mn::Lda
                || a2.mn != op
                || s2.mn != Mn::Sta
            {
                continue;
            }
            let (Some(lo), Some(lo2), Some(hi), Some(hi2)) = (mem(l1), mem(s1), mem(l2), mem(s2))
            else {
                continue;
            };
            let (Some(add_lo), Some(add_hi)) = (opnd(a1), opnd(a2)) else {
                continue;
            };
            if lo != lo2 || hi != hi2 {
                continue;
            }
            // A third byte: `ADC m; STA m` within the next six.
            let after = v.iter().position(|i| i.pc == s2.pc).unwrap_or(0);
            let mut top = None;
            for k in after + 1..v.len().min(after + 7) {
                if v[k].mn == Mn::Adc
                    && let (Some(m), Some(n)) = (mem(&v[k]), v.get(k + 1).and_then(mem))
                    && m == n
                    && v[k + 1].mn == Mn::Sta
                {
                    top = Some(m);
                    break;
                }
                if matches!(v[k].mn, Mn::Rts | Mn::Jmp | Mn::Jsr | Mn::Clc | Mn::Sec) {
                    break;
                }
            }
            out.push(Chain {
                pc: l1.pc,
                sub,
                lo,
                hi,
                add_lo,
                add_hi,
                top,
            });
        }
        out
    }

    /// Every loop: a branch back, and how it counts.
    fn loops(&self) -> Vec<Loop> {
        let v = self.all_insns();
        let mut out = Vec::new();
        for (k, b) in v.iter().enumerate() {
            if b.mode != Mode::Rel || b.arg >= b.pc {
                continue;
            }
            let head = b.arg;
            let prev = |n: usize| if k >= n { Some(&v[k - n]) } else { None };
            let mut lp = Loop {
                head,
                back: b.pc,
                reg: None,
                count: None,
                by_byte: None,
                next: b.pc,
            };
            match (prev(2), prev(1)) {
                (Some(i), Some(c))
                    if matches!(i.mn, Mn::Inx | Mn::Iny)
                        && matches!(c.mn, Mn::Cpx | Mn::Cpy)
                        && c.mode == Mode::Imm =>
                {
                    lp.reg = Some(if i.mn == Mn::Inx { Reg::X } else { Reg::Y });
                    lp.count = Some(c.arg as u16);
                    lp.next = i.pc;
                }
                (_, Some(d)) if matches!(d.mn, Mn::Dex | Mn::Dey) && b.mn == Mn::Bpl => {
                    let reg = if d.mn == Mn::Dex { Reg::X } else { Reg::Y };
                    lp.reg = Some(reg);
                    lp.next = d.pc;
                    let ld = if reg == Reg::X { Mn::Ldx } else { Mn::Ldy };
                    if let Some(init) = v[..k]
                        .iter()
                        .rev()
                        .skip_while(|i| i.pc >= head)
                        .take(4)
                        .find(|i| i.mn == ld)
                        && init.mode == Mode::Imm
                    {
                        lp.count = Some(init.arg as u16 + 1);
                    }
                }
                (_, Some(d))
                    if d.mn == Mn::Dec
                        && b.mn == Mn::Bne
                        && matches!(d.mode, Mode::Zp | Mode::Abs) =>
                {
                    lp.by_byte = Some(Loc::at(d.arg));
                    lp.next = d.pc;
                }
                (Some(l), Some(c)) if l.mn == Mn::Lda && c.mn == Mn::Cmp && c.mode == Mode::Imm => {
                    // INC m; LDA m; CMP #n, with the register loaded from m
                    // in the loop.
                    if let Some(i) = prev(3)
                        && i.mn == Mn::Inc
                        && i.arg == l.arg
                    {
                        lp.next = i.pc;
                        lp.count = Some(c.arg as u16);
                        let body = &v[..k];
                        if body.iter().any(|x| {
                            x.pc >= head && x.mn == Mn::Ldx && x.arg == l.arg && x.mode == l.mode
                        }) {
                            lp.reg = Some(Reg::X);
                        } else if body.iter().any(|x| {
                            x.pc >= head && x.mn == Mn::Ldy && x.arg == l.arg && x.mode == l.mode
                        }) {
                            lp.reg = Some(Reg::Y);
                        }
                    }
                }
                _ => {}
            }
            out.push(lp);
        }
        out
    }
}

/// Whether a step is a high byte's: under its own routine's test that
/// another byte of the same kind just wrapped (an increment that made it 0,
/// or a 0 read before it is decremented).
fn wraps(loc: Loc, by: i8, guards: &BTreeSet<Guard>) -> bool {
    use super::absint::Flags;
    guards.iter().any(|g| {
        let Flags::Res {
            op,
            src: Some(l),
            mask: None,
            ..
        } = &g.test
        else {
            return false;
        };
        let zero = matches!((g.op, g.taken), (Mn::Beq, true) | (Mn::Bne, false));
        let fits = if by > 0 {
            *op == Mn::Inc
        } else {
            matches!(op, Mn::Lda | Mn::Ldx | Mn::Ldy)
        };
        zero && fits && *l != loc && l.idx == loc.idx
    })
}

/// What the rules found, kept between them.
#[derive(Default)]
struct Found {
    /// The sprite table's first byte: the page the code copies to the PPU
    /// by DMA (the value it writes to $4014), else $0200.
    oam: Va,
    frame: Option<Loc>,
    pad: Option<Loc>,
    pad_first_high: bool,
    pad_new: Option<Loc>,
    pad_prev: Option<Loc>,
    pad_routine: Option<Va>,
    scratch: BTreeSet<Loc>,
    states: BTreeMap<Loc, BTreeSet<u8>>,
    game: Option<Loc>,
    counters: BTreeSet<Loc>,
    timers: BTreeSet<Loc>,
    /// Slot groups: name, count, arrays.
    groups: Vec<(String, u16, BTreeSet<Va>)>,
    player: BTreeSet<Loc>,
    px: Option<Loc>,
    px_hi: Option<Loc>,
    py: Option<Loc>,
    vy: Option<(Loc, Loc)>,
    cells: BTreeSet<Va>,
    camera: Option<Loc>,
    /// The slot group's x array, hi array, state array.
    gx: Option<Va>,
    gstate: Option<Loc>,
    /// The game state's value for the death, set with the player's speed.
    dying: Option<u8>,
    initial: Option<u8>,
    hitbox: Option<(i32, i32, i32, i32)>,
    group_y: Option<i32>,
    /// The activations that look up a cell at a point around the player.
    probe_acts: BTreeSet<usize>,
}

fn is_pad_cond(f: &Found, c: &Cond) -> bool {
    matches!(c.subj, Subj::Mask(l, _) if Some(l) == f.pad || Some(l) == f.pad_new)
}

/// Runs every rule.
pub(super) fn run(h: &mut H) {
    let mut f = Found::default();
    basics(h, &mut f);
    states(h, &mut f);
    dispatch(h, &mut f);
    tables(h);
    if f.game.is_none() {
        // No compare chain calls routines by it: the state byte the frame
        // loop itself tests by the most values.
        let loops: BTreeSet<usize> =
            h.r.acts
                .iter()
                .enumerate()
                .filter(|(_, a)| h.r.loops.contains(&a.start))
                .map(|(i, _)| i)
                .collect();
        let mut best: Option<(usize, Loc)> = None;
        for (s, _) in f.states.iter().filter(|(s, _)| s.idx.is_none()) {
            let mut vals = BTreeSet::new();
            for &li in &loops {
                for e in &h.r.acts[li].events {
                    if let Ev::Branch { pc, op, test, .. } = e
                        && let Some(c) = cond::of(&Guard {
                            pc: *pc,
                            op: *op,
                            taken: true,
                            test: test.clone(),
                        })
                        && c.byte() == Some(*s)
                    {
                        vals.insert(c.k);
                    }
                }
            }
            if vals.len() >= 2 && best.is_none_or(|b| vals.len() > b.0) {
                best = Some((vals.len(), *s));
            }
        }
        f.game = best.map(|b| b.1);
    }
    groups(h, &mut f);
    player(h, &mut f);
    probes(h, &mut f);
    moves(h, &mut f);
    camera(h, &mut f);
    machines(h, &mut f);
    contact(h, &mut f);
    level(h, &mut f);
    counters(h, &mut f);
    elements(h, &mut f);
    shape(h, &f);
    constants(h, &f);
    h.out.facts.sort_by(|a, b| a.path.cmp(&b.path));
    h.out.open.sort_by(|a, b| a.what.cmp(&b.what));
}

/// The frame counter, the controller bytes, scratch.
fn basics(h: &mut H, f: &mut Found) {
    // The sprite table: the page written to the DMA register. One page if
    // the code names one; the usual $0200 when it names none or several.
    let pages: BTreeSet<u8> = h
        .events()
        .filter_map(|(_, e)| match e {
            Ev::Write { loc, val, .. } if *loc == Loc::at(0x4014) => val.konst(),
            _ => None,
        })
        .filter(|&p| p < 0x08)
        .collect();
    f.oam = match pages.iter().next() {
        Some(&p) if pages.len() == 1 => u32::from(p) << 8,
        _ => 0x0200,
    };
    let nmi = u32::from(h.code.word(0xFFFA));
    let mut frame = None;
    for (i, e) in h.events() {
        if let Ev::Step {
            loc,
            by: 1,
            guards,
            pc,
        } = e
            && h.r.acts[i].start == nmi
            && guards.is_empty()
            && loc.ram()
            && loc.idx.is_none()
        {
            frame = Some((*loc, *pc));
            break;
        }
    }
    if let Some((l, pc)) = frame {
        f.frame = Some(l);
        h.fact(
            "hardware.frame_counter",
            json!(hx(&l)),
            "a byte the NMI adds 1 to on every interrupt",
            &[pc],
        );
    }
    // The controller: the act that reads $4016 and shifts its bits in.
    for (i, a) in h.r.acts.iter().enumerate() {
        if !a
            .events
            .iter()
            .any(|e| matches!(e, Ev::Read { loc, .. } if loc.addr == 0x4016))
        {
            continue;
        }
        // Controller 1's byte: the one shifted in from $4016's bits, not
        // $4017's (controller 2).
        for e in &a.events {
            if let Ev::Write { pc, loc, val, .. } = e
                && let Some(ins) = h.code.insns.get(pc)
                && matches!(ins.mn, Mn::Rol | Mn::Ror)
                && loc.ram()
                && val.places().iter().any(|l| l.addr == 0x4016)
                && !val.places().iter().any(|l| l.addr == 0x4017)
                && f.pad.is_none_or(|p| p == *loc)
            {
                f.pad = Some(*loc);
                f.pad_first_high = ins.mn == Mn::Rol;
                f.pad_routine = Some(a.start);
            }
        }
        let Some(p) = f.pad else { continue };
        for e in &a.events {
            if let Ev::Write { loc, val, .. } = e
                && *loc != p
                && loc.ram()
                && val.lin() == Some((Some(p), None, 0))
            {
                f.pad_prev = Some(*loc);
            }
        }
        for e in &a.events {
            if let Ev::Write { loc, val, .. } = e
                && *loc != p
                && Some(*loc) != f.pad_prev
                && loc.ram()
            {
                let ps = kind_places(val);
                if ps.contains(&p) && f.pad_prev.is_some_and(|q| ps.contains(&q)) {
                    f.pad_new = Some(*loc);
                }
            }
        }
        let _ = i;
        break;
    }
    if let (Some(p), Some(rt)) = (f.pad, f.pad_routine) {
        let order = if f.pad_first_high {
            "a in bit 7"
        } else {
            "a in bit 0"
        };
        h.fact(
            "input.pad",
            json!(hx(&p)),
            &format!("the byte the controller's bits are shifted into, {order}"),
            &[rt],
        );
        if let Some(n) = f.pad_new {
            h.fact(
                "input.pad_new",
                json!(hx(&n)),
                "the pad with the last frame's buttons masked out",
                &[rt],
            );
        }
    }
    // Scratch: no read of it in any activation sees what the last frame
    // left.
    let mut live: BTreeSet<Loc> = BTreeSet::new();
    let mut seen: BTreeSet<Loc> = BTreeSet::new();
    for (i, e) in h.events() {
        if h.r.acts[i].general {
            continue;
        }
        match e {
            Ev::Read { loc, fresh, .. } if loc.ram() => {
                seen.insert(*loc);
                if *fresh {
                    live.insert(*loc);
                }
            }
            Ev::Write { loc, .. } | Ev::Step { loc, .. } if loc.ram() => {
                seen.insert(*loc);
            }
            _ => {}
        }
    }
    f.scratch = seen
        .difference(&live)
        .copied()
        .filter(|l| l.addr < 0x0100 || l.idx.is_none())
        .collect();
}

/// State bytes: tested by value, written by constant.
fn states(h: &mut H, f: &mut Found) {
    let mut tested: BTreeMap<Loc, BTreeSet<u8>> = BTreeMap::new();
    let mut written: BTreeMap<Loc, BTreeSet<u8>> = BTreeMap::new();
    let mut stepped: BTreeSet<Loc> = BTreeSet::new();
    let mut step_tested: BTreeSet<Loc> = BTreeSet::new();
    let mut cmp_any: BTreeSet<Loc> = BTreeSet::new();
    let mut ups: BTreeSet<Loc> = BTreeSet::new();
    let mut other: BTreeSet<Loc> = BTreeSet::new();
    for (i, e) in h.events() {
        match e {
            Ev::Branch { op, test, pc, .. } => {
                let g = Guard {
                    pc: *pc,
                    op: *op,
                    taken: true,
                    test: test.clone(),
                };
                if let Some(c) = cond::of(&g) {
                    // A compare in the routine that reads the byte (not one
                    // a callee makes with the value, such as a digit split).
                    if let Subj::Byte(l, _) = c.subj
                        && c.k != 0
                        && h.r.acts[i]
                            .events
                            .iter()
                            .any(|x| matches!(x, Ev::Read { loc, .. } if *loc == l))
                    {
                        cmp_any.insert(l);
                    }
                    if let Subj::Byte(l, off) = c.subj
                        && matches!(c.rel, Rel::Eq | Rel::Ne)
                    {
                        if off == 0 {
                            tested.entry(l).or_default().insert(c.k);
                        } else {
                            step_tested.insert(l);
                        }
                    }
                    if let super::absint::Flags::Res {
                        op: Mn::Dec | Mn::Inc,
                        src: Some(l),
                        ..
                    } = test
                    {
                        step_tested.insert(*l);
                    }
                }
            }
            // A jump table tests its index byte against each entry's value.
            Ev::Indirect { lo, hi, rts, .. } => {
                for (_, v) in super::absint::table_cases(h.code, lo, hi, *rts) {
                    if let Some((l, k)) = v {
                        tested.entry(l).or_default().insert(k);
                    }
                }
            }
            Ev::Write { loc, val, .. } => match val.konst() {
                Some(k) => {
                    written.entry(*loc).or_default().insert(k);
                }
                None => {
                    other.insert(*loc);
                }
            },
            Ev::Step { loc, by, .. } => {
                stepped.insert(*loc);
                if *by > 0 {
                    ups.insert(*loc);
                }
            }
            _ => {}
        }
    }
    // A high byte: stepped only when another byte wraps to 0.
    let mut high: BTreeSet<Loc> = BTreeSet::new();
    let mut low: BTreeSet<Loc> = BTreeSet::new();
    for (_, e) in h.events() {
        if let Ev::Step {
            loc, guards, by, ..
        } = e
        {
            if wraps(*loc, *by, guards) {
                high.insert(*loc);
            } else {
                low.insert(*loc);
            }
        }
    }
    high.retain(|l| !low.contains(l));
    for (l, w) in &written {
        if !l.ram()
            || stepped.contains(l)
            || other.contains(l)
            || Some(*l) == f.frame
            || f.scratch.contains(l)
        {
            continue;
        }
        let t = tested.get(l).cloned().unwrap_or_default();
        let all: BTreeSet<u8> = t.union(w).copied().collect();
        if t.is_empty() || w.len() < 2 || !(t.iter().any(|&k| k != 0) || all.len() >= 3) {
            continue;
        }
        f.states.insert(*l, all);
    }
    // Counters: changed by +1 or -1 and constants, tested only against 0.
    for l in &stepped {
        if !l.ram()
            || Some(*l) == f.frame
            || other.contains(l)
            || f.scratch.contains(l)
            || l.idx.is_some()
            || high.contains(l)
        {
            continue;
        }
        if step_tested.contains(l) && !ups.contains(l) {
            f.timers.insert(*l);
            continue;
        }
        // A counter counts one way; a byte stepped both ways is a place.
        let downs = h
            .events()
            .any(|(_, e)| matches!(e, Ev::Step { loc, by, .. } if loc == l && *by < 0));
        if cmp_any.contains(l) || step_tested.contains(l) || (ups.contains(l) && downs) {
            continue;
        }
        f.counters.insert(*l);
    }
    for l in &stepped {
        if step_tested.contains(l) && !ups.contains(l) && l.idx.is_some() {
            f.timers.insert(*l);
        }
    }
}

fn state_name(v: u8) -> String {
    format!("s{v}")
}

fn byte_name(l: &Loc) -> String {
    format!("state_{:02X}", l.addr)
}

/// Compare chains in the frame loop: which routines run for each value of
/// a state byte.
fn dispatch(h: &mut H, f: &mut Found) {
    let loops: Vec<usize> =
        h.r.acts
            .iter()
            .enumerate()
            .filter(|(_, a)| h.r.loops.contains(&a.start))
            .map(|(i, _)| i)
            .collect();
    let states = f.states.clone();
    for (s, values) in &states {
        if s.idx.is_some() {
            continue;
        }
        let mut cases: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut first = None;
        for &li in &loops {
            for e in &h.r.acts[li].events {
                let Ev::Call { to, guards, pc, .. } = e else {
                    continue;
                };
                let cs: Vec<Cond> = guards.iter().filter_map(cond::of).collect();
                let mut vals = values.clone();
                let mut any = false;
                for c in &cs {
                    if let Some((l, k)) = c.equals()
                        && l == *s
                    {
                        vals.retain(|&v| v == k);
                        any = true;
                        first = first.min(Some(c.pc)).or(Some(c.pc));
                    }
                    if let Some((l, k)) = c.differs()
                        && l == *s
                    {
                        vals.remove(&k);
                        any = true;
                    }
                }
                if !any {
                    continue;
                }
                let key = vals
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let _ = pc;
                cases.entry(key).or_default().insert(h.rname(*to));
            }
        }
        if cases.len() >= 2 {
            f.game.get_or_insert(*s);
            h.out.dispatch.push(Dispatch {
                kind: "compare chain".into(),
                at: first.map(crate::codemap::va_hex).unwrap_or_default(),
                subject: hx(s),
                cases: cases
                    .into_iter()
                    .map(|(k, v)| (k, v.into_iter().collect()))
                    .collect(),
            });
        }
    }
}

/// The largest value a table's index byte takes, when every write that can
/// reach it (its own, and indexed writes whose array covers it) is a
/// constant and nothing steps it: a state byte's last state.
fn index_max(h: &H, by: Option<Loc>) -> Option<u8> {
    let l = by?;
    let reaches = |loc: &Loc| {
        *loc == l || (loc.idx.is_some() && loc.addr <= l.addr && l.addr - loc.addr < 0x100)
    };
    let mut max: Option<u8> = None;
    for (_, e) in h.events() {
        match e {
            Ev::Write { loc, val, .. } if reaches(loc) => {
                let k = val.konst()?;
                max = Some(max.map_or(k, |m| m.max(k)));
            }
            Ev::Step { loc, .. } if reaches(loc) => return None,
            _ => {}
        }
    }
    max
}

/// Jump tables: an indirect jump through a pointer loaded from two ROM
/// tables at one index, or an `RTS` to an address pushed from them.
fn tables(h: &mut H) {
    let mut found = Vec::new();
    for (_, e) in h.events() {
        let Ev::Indirect { pc, lo, hi, rts } = e else {
            continue;
        };
        let (Some(a), Some(b)) = (lo.table_or_from(), hi.table_or_from()) else {
            let ps: Vec<Loc> = lo.places().union(&hi.places()).copied().collect();
            found.push((*pc, None, ps));
            continue;
        };
        if !(a.rom() && b.rom() && a.idx.is_some() && a.idx == b.idx) {
            found.push((*pc, None, vec![a, b]));
            continue;
        }
        let cases = super::absint::table_cases(h.code, lo, hi, *rts);
        let by = lo.table.and_then(|t| t.by).map(|(l, _)| l);
        found.push((*pc, Some((a, b, cases, by)), vec![]));
    }
    // A site read with its table resolved, before one read without.
    found.sort_by_key(|f| (f.0, f.1.is_none()));
    found.dedup_by_key(|f| f.0);
    // Where each table starts: another site's table ends one that runs
    // into it.
    let starts: BTreeSet<Va> = found
        .iter()
        .filter_map(|f| f.1.as_ref())
        .flat_map(|(a, b, ..)| [a.addr, b.addr])
        .collect();
    let found: Vec<_> = found
        .into_iter()
        .map(|(pc, t, ps)| {
            let t = t.map(|(a, b, cases, by)| {
                let split = b.addr != a.addr.wrapping_add(1);
                let ends = |from: Va, own: &[Va]| {
                    starts
                        .range(from.wrapping_add(1)..)
                        .find(|s| !own.contains(s))
                        .copied()
                        .unwrap_or(Va::MAX)
                };
                let (end_lo, end_hi) = (ends(a.addr, &[b.addr]), ends(b.addr, &[a.addr]));
                let max = index_max(h, by);
                let entries: Vec<Va> = cases
                    .into_iter()
                    .enumerate()
                    .take_while(|(i, _)| {
                        let i = *i as u16;
                        if split {
                            a.addr.saturating_add(u32::from(i)) < end_lo
                                && b.addr.saturating_add(u32::from(i)) < end_hi
                        } else {
                            a.addr.saturating_add(u32::from(2 * i + 1)) < end_lo
                        }
                    })
                    .filter(|(_, (_, v))| match (max, v) {
                        (Some(m), Some((_, k))) => *k <= m,
                        _ => true,
                    })
                    .map(|(_, (t, _))| t)
                    .collect();
                (a, entries, by)
            });
            (pc, t, ps)
        })
        .collect();
    for (pc, t, ps) in found {
        match t {
            Some((a, entries, by)) => {
                h.tables.extend(entries.iter().map(|&e| (pc, e)));
                let mut index = h
                    .events()
                    .find_map(|(_, e)| match e {
                        Ev::Read {
                            loc,
                            index: Some(iv),
                            ..
                        } if *loc == a => Some(iv.places()),
                        _ => None,
                    })
                    .unwrap_or_default();
                if index.is_empty() {
                    index.extend(by);
                }
                let mut cases = BTreeMap::new();
                for (i, t) in entries.iter().enumerate() {
                    cases.insert(format!("{i:03}"), vec![crate::codemap::routine_name(*t)]);
                }
                h.out.dispatch.push(Dispatch {
                    kind: "jump table".into(),
                    at: crate::codemap::va_hex(pc),
                    subject: index.iter().map(hx).collect::<Vec<_>>().join(" "),
                    cases,
                });
            }
            None => h.open(
                format!("jump at {}", crate::codemap::va_hex(pc)),
                Some(pc),
                &ps,
                "an indirect jump whose pointer isn't read from ROM tables at one index",
            ),
        }
    }
}

/// Slot groups: RAM arrays a loop indexes with its counter.
fn groups(h: &mut H, f: &mut Found) {
    let loops = h.loops();
    let mut sets: Vec<(u16, BTreeSet<Va>)> = Vec::new();
    for lp in &loops {
        let (Some(reg), Some(n)) = (lp.reg, lp.count) else {
            continue;
        };
        if !(2..=32).contains(&n) {
            continue;
        }
        let mut arrays = BTreeSet::new();
        for (_, e) in h.events() {
            let (pc, loc) = match e {
                Ev::Read { pc, loc, .. } | Ev::Write { pc, loc, .. } | Ev::Step { pc, loc, .. } => {
                    (*pc, *loc)
                }
                _ => continue,
            };
            let Some(ins) = h.code.insns.get(&pc) else {
                continue;
            };
            if pc < lp.head || pc > lp.back || matches!(ins.mode, Mode::IndY | Mode::IndX) {
                continue;
            }
            if loc.idx == Some(reg)
                && loc.ram()
                && !(f.oam..f.oam + 0x100).contains(&loc.addr)
                && !f.scratch.contains(&loc)
            {
                arrays.insert(loc.addr);
            }
        }
        // An array per slot field: neighbours n apart. The loop's count is
        // the group's count fact.
        if arrays.len() >= 2 {
            sets.push((n, arrays));
            h.cited.insert(lp.back);
            if let Some((p, _)) = h.code.insns.range(..lp.back).next_back() {
                h.cited.insert(*p);
            }
        }
    }
    // Merge sets that share an array.
    let mut merged: Vec<(u16, BTreeSet<Va>)> = Vec::new();
    for (n, s) in sets {
        if let Some(m) = merged
            .iter_mut()
            .find(|(k, m)| *k == n && !m.is_disjoint(&s))
        {
            m.1.extend(s);
        } else {
            merged.push((n, s));
        }
    }
    for (n, s) in merged {
        let base = *s.iter().next().unwrap_or(&0);
        let name = format!("slots_{base:02X}");
        f.groups.push((name, n, s));
    }
    f.groups.sort();
}

/// The controlled object: what the code writes under a button test.
fn player(h: &mut H, f: &mut Found) {
    if f.pad.is_none() {
        h.open("player", None, &[], "no controller read found");
        return;
    }
    let mut w: BTreeSet<Loc> = BTreeSet::new();
    let mut steps: BTreeMap<Loc, BTreeSet<(i8, u8)>> = BTreeMap::new();
    let mut evs = Vec::new();
    for (i, e) in h.events() {
        let (loc, own, by) = match e {
            Ev::Write { loc, guards, .. } => (*loc, guards, 0),
            Ev::Step {
                loc, guards, by, ..
            } => (*loc, guards, *by),
            _ => continue,
        };
        if !loc.ram()
            || loc.idx.is_some()
            || f.scratch.contains(&loc)
            || f.states.contains_key(&loc)
        {
            continue;
        }
        evs.push((i, loc, own.clone(), by));
    }
    for (i, loc, own, by) in evs {
        // A write is the player's when its own routine tests the button; a
        // step when any caller on the way did (a routine that moves one
        // pixel, called once per pixel).
        let own_pad = own.iter().filter_map(cond::of).any(|c| is_pad_cond(f, &c));
        for p in h.paths(i, &own) {
            if let Some(c) = p.iter().find(|c| is_pad_cond(f, c)) {
                if by == 0 && !own_pad {
                    continue;
                }
                w.insert(loc);
                if by != 0
                    && let Subj::Mask(_, m) = c.subj
                {
                    steps.entry(loc).or_default().insert((by, m));
                }
            }
        }
    }
    // x: stepped both ways under two different buttons.
    f.px = steps
        .iter()
        .find(|(_, s)| s.iter().any(|x| x.0 > 0) && s.iter().any(|x| x.0 < 0))
        .map(|(l, _)| *l);
    // Its high byte: stepped when the low byte wraps.
    if let Some(x) = f.px {
        for (i, e) in h.events() {
            if let Ev::Step { loc, guards, .. } = e
                && *loc != x
            {
                let wraps = h.paths(i, guards).iter().any(|p| {
                    p.iter().any(|c| {
                        matches!(c.subj, Subj::Byte(l, _) if l == x) && c.rel == Rel::Eq && c.k == 0
                    })
                });
                if wraps && loc.idx.is_none() {
                    f.px_hi = Some(*loc);
                    w.insert(*loc);
                    break;
                }
            }
        }
    }
    // y: the position a speed written under a button moves.
    for c in h.chains16() {
        if let (Ok(al), Ok(ah)) = (c.add_lo, c.add_hi)
            && !c.sub
            && w.contains(&ah)
            && Some(c.hi) != f.px
        {
            f.py = Some(c.hi);
            f.vy = Some((ah, al));
            w.insert(c.hi);
            w.insert(c.lo);
            w.insert(al);
            w.extend(c.top);
        }
    }
    w.retain(|l| Some(*l) != f.pad && Some(*l) != f.pad_new && Some(*l) != f.pad_prev);
    f.player = w;
    let mut pcs = Vec::new();
    if let Some(rt) = f.pad_routine {
        pcs.push(rt);
    }
    if let Some(x) = f.px {
        h.fact(
            "entities.player.bytes.x",
            json!(hx(&x)),
            "stepped both ways under the controller's direction buttons",
            &pcs,
        );
    }
    if let Some(y) = f.py {
        h.fact(
            "entities.player.bytes.y",
            json!(hx(&y)),
            "the position the speed set under a button is added to",
            &pcs,
        );
    }
}

/// Tile probes: the points around the player where the code looks up the
/// level's cells, and what it does with each.
fn probes(h: &mut H, f: &mut Found) {
    let (Some(px), Some(py)) = (f.px, f.py) else {
        h.open(
            "entities.player.hitbox",
            None,
            &[],
            "the player's x and y bytes aren't known",
        );
        return;
    };
    // The level's cells: RAM arrays whose value indexes an asset table.
    let mut prop_reads: Vec<(usize, Va, String, Loc)> = Vec::new();
    for (i, e) in h.events() {
        if let Ev::Read {
            pc,
            loc,
            index: Some(iv),
            ..
        } = e
            && loc.rom()
            && let Some(t) = h.table_of(loc.addr)
            && t.asset_kind == "metatiles"
        {
            for p in kind_places(iv) {
                if p.ram() && p.idx.is_some() && p.addr >= 0x0100 {
                    f.cells.insert(p.addr);
                }
            }
            prop_reads.push((i, *pc, t.column.clone(), *loc));
        }
    }
    if f.cells.is_empty() {
        h.open(
            "entities.player.hitbox",
            None,
            &[],
            "no level cells found: no metatile property is read by a cell's value",
        );
        return;
    }
    // A lookup: an activation reading the cells; its arguments are the
    // non-indexed scratch bytes it reads as the player's x or y plus a
    // constant.
    #[derive(Debug, Clone)]
    struct Probe {
        dx: i16,
        dy: i16,
        act: usize,
        pc: Va,
    }
    let mut probes = Vec::new();
    for (i, a) in h.r.acts.iter().enumerate() {
        let reads_cells = a.events.iter().any(|e| matches!(e, Ev::Read { loc, .. } if loc.idx.is_some() && f.cells.contains(&loc.addr)));
        if !reads_cells {
            continue;
        }
        let (mut dx, mut dy) = (None, None);
        let mut pc0 = a.start;
        for e in &a.events {
            if let Ev::Read { loc, val, pc, .. } = e
                && loc.idx.is_none()
                && loc.ram()
                && Some(*loc) != f.px
                && Some(*loc) != f.py
                && let Some((Some(p), None, off)) = val.lin()
            {
                if p == px && dx.is_none() {
                    dx = Some(off);
                    pc0 = *pc;
                }
                if p == py && dy.is_none() {
                    dy = Some(off);
                }
            }
        }
        if let (Some(dx), Some(dy)) = (dx, dy) {
            f.probe_acts.insert(i);
            probes.push(Probe {
                dx,
                dy,
                act: i,
                pc: pc0,
            });
        }
    }
    let vy = f.vy.map(|v| v.0);
    let mut side: BTreeSet<(i16, i16)> = BTreeSet::new();
    let mut feet: BTreeSet<(i16, i16)> = BTreeSet::new();
    let mut head: BTreeSet<(i16, i16)> = BTreeSet::new();
    let mut other: Vec<(i16, i16, usize)> = Vec::new();
    let mut pcs = Vec::new();
    for p in &probes {
        let paths = h.paths(p.act, &BTreeSet::new());
        for cs in &paths {
            let pad = cs.iter().any(|c| is_pad_cond(f, c));
            let rising = cs
                .iter()
                .any(|c| matches!(c.subj, Subj::Byte(l, 0) if Some(l) == vy) && c.rel == Rel::Neg);
            let falling = cs.iter().any(|c| {
                matches!(c.subj, Subj::Byte(l, 0) if Some(l) == vy) && c.rel == Rel::NonNeg
            });
            if pad {
                side.insert((p.dx, p.dy));
            } else if falling {
                feet.insert((p.dx, p.dy));
            } else if rising {
                head.insert((p.dx, p.dy));
            } else {
                other.push((p.dx, p.dy, p.act));
            }
        }
        pcs.push(p.pc);
    }
    // The box: x and width from the feet, y and height from the sides.
    if feet.len() >= 2 && side.len() >= 2 {
        let x0 = feet.iter().map(|p| p.0).min().unwrap_or(0);
        let x1 = feet.iter().map(|p| p.0).max().unwrap_or(0);
        let y0 = side.iter().map(|p| p.1).min().unwrap_or(0);
        let y1 = side.iter().map(|p| p.1).max().unwrap_or(0);
        let foot = feet.iter().map(|p| p.1).max().unwrap_or(0);
        if foot == y1 + 1 {
            let (x, w, y, hh) = (
                i32::from(x0),
                i32::from(x1 - x0 + 1),
                i32::from(y0),
                i32::from(y1 - y0 + 1),
            );
            f.hitbox = Some((x, w, y, hh));
            let r = "the tile probes: x and width from the feet (checked while falling), y and height from the sides (checked under the direction buttons)";
            h.fact("entities.player.hitbox.x", json!(x), r, &pcs);
            h.fact("entities.player.hitbox.w", json!(w), r, &pcs);
            h.fact("entities.player.hitbox.y", json!(y), r, &pcs);
            h.fact("entities.player.hitbox.h", json!(hh), r, &pcs);
        } else {
            h.open(
                "entities.player.hitbox",
                None,
                &[px, py],
                format!("the feet probes ({foot}) aren't one below the side probes ({y1})"),
            );
        }
    } else {
        h.open(
            "entities.player.hitbox",
            None,
            &[px, py],
            "too few feet or side probes",
        );
    }
    if head.len() == 1 {
        let (x, y) = *head.iter().next().unwrap_or(&(0, 0));
        h.fact(
            "entities.player.jump.head.point",
            json!([x, y]),
            "the probe checked only while rising",
            &pcs,
        );
    } else if !head.is_empty() {
        h.open(
            "entities.player.jump.head.point",
            None,
            &[px, py],
            "more than one probe while rising",
        );
    }
    // Probes whose cell's kind is compared with a constant: what happens.
    let kinds = tile_kinds(h, f);
    let mut pickups: BTreeSet<(i16, i16)> = BTreeSet::new();
    for (dx, dy, act) in &other {
        // The kind compare is in this probe's caller.
        let callers: Vec<usize> = h.r.acts[*act].callers.iter().map(|c| c.0).collect();
        for c in callers {
            if kinds.iter().any(|k| k.act == c) {
                pickups.insert((*dx, *dy));
            }
        }
    }
    if !pickups.is_empty() {
        let pts: Vec<Value> = pickups.iter().map(|(x, y)| json!([x, y])).collect();
        h.fact(
            "entities.player.jump.pickup.points",
            json!(pts),
            "probes run every frame whose cell's kind is compared",
            &pcs,
        );
    }
    // The tile kinds the head probe and the pickup look for.
    for k in &kinds {
        let rising = h.paths(k.act, &BTreeSet::new()).iter().any(|cs| {
            cs.iter()
                .any(|c| matches!(c.subj, Subj::Byte(l, 0) if Some(l) == vy) && c.rel == Rel::Neg)
        });
        let base = if rising {
            "entities.player.jump.head.hits[0]"
        } else {
            "entities.player.jump.pickup"
        };
        h.fact(
            format!("{base}.kind"),
            json!(k.kind),
            "the kind the routine compares the cell's kind with",
            &[k.pc],
        );
        if let Some(b) = k.becomes {
            h.fact(
                format!("{base}.becomes_metatile"),
                json!(b),
                "the metatile the cell is set to on that branch",
                &[k.pc],
            );
            if let Some(&solid) = h.inp.metatile_solid.get(usize::from(b)) {
                let kind = h
                    .inp
                    .metatile_kind
                    .get(usize::from(b))
                    .copied()
                    .unwrap_or(0);
                let what = if solid & 1 != 0 {
                    "solid"
                } else if kind == 0 {
                    "empty"
                } else {
                    ""
                };
                if !what.is_empty() {
                    h.fact(
                        format!("{base}.becomes"),
                        json!(what),
                        "the new metatile's solid property, and no kind",
                        &[k.pc],
                    );
                }
            }
        }
        if rising {
            // The speed the head hit leaves: the speed written in the same
            // routine when the cell is solid.
            if let Some((hi, lo)) = f.vy {
                let mut val = None;
                let mut spcs = Vec::new();
                for e in &h.r.acts[k.act].events {
                    if let Ev::Write {
                        loc, val: v, pc, ..
                    } = e
                        && (*loc == hi || *loc == lo)
                        && let Some(c) = v.konst()
                    {
                        val = Some((
                            val.map_or(0, |x: (u16, Va)| x.0)
                                | (u16::from(c) << if *loc == hi { 8 } else { 0 }),
                            *pc,
                        ));
                        spcs.push(*pc);
                    }
                }
                if let Some((v, _)) = val {
                    h.fact(
                        "entities.player.jump.head.hits[0].speed",
                        json!(f64::from(v as i16) / 256.0),
                        "the speed written when the cell over the head is solid",
                        &spcs,
                    );
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct TileKind {
    act: usize,
    pc: Va,
    kind: u8,
    becomes: Option<u8>,
}

/// Each compare of a cell's kind with a constant, and the metatile the cell
/// becomes on the equal branch.
fn tile_kinds(h: &H, f: &Found) -> Vec<TileKind> {
    let mut out: Vec<TileKind> = Vec::new();
    for (i, e) in h.events() {
        let Ev::Branch {
            pc,
            test: super::absint::Flags::Cmp { lhs, rhs },
            ..
        } = e
        else {
            continue;
        };
        let (Some(l), Some(k)) = (lhs.from, rhs.konst()) else {
            continue;
        };
        if !l.rom() || h.table_of(l.addr).is_none_or(|t| t.column != "kind") {
            continue;
        }
        if out.iter().any(|t| t.act == i && t.kind == k) {
            continue;
        }
        // The cell write under `kind == k`, in an activation this one calls.
        let mut becomes = None;
        for (j, e2) in h.events() {
            if let Ev::Write {
                loc, val, guards, ..
            } = e2
                && loc.idx.is_some()
                && f.cells.contains(&loc.addr)
                && let Some(v) = val.konst()
                && h.r.acts[j].callers.iter().any(|c| c.0 == i)
            {
                let ok = h.paths(j, guards).iter().any(|cs| {
                    cs.iter()
                        .any(|c| c.pc == *pc && c.rel == Rel::Eq && c.k == k)
                });
                if ok {
                    becomes = Some(v);
                }
            }
        }
        out.push(TileKind {
            act: i,
            pc: *pc,
            kind: k,
            becomes,
        });
    }
    out
}

/// Speeds: the jump, gravity, the fall limit, walking and running.
fn moves(h: &mut H, f: &mut Found) {
    let Some((vh, vl)) = f.vy else {
        h.open("entities.player.jump", None, &[], "no vertical speed found");
        return;
    };
    // The jump: a 16-bit constant written to the speed under a new press.
    let mut found_jump = false;
    let mut evs: Vec<(usize, Va, Loc, u8, BTreeSet<Guard>)> = Vec::new();
    for (i, e) in h.events() {
        if let Ev::Write {
            pc,
            loc,
            val,
            guards,
            ..
        } = e
            && (*loc == vh || *loc == vl)
            && let Some(k) = val.konst()
        {
            evs.push((i, *pc, *loc, k, guards.clone()));
        }
    }
    let pair =
        |evs: &[(usize, Va, Loc, u8, BTreeSet<Guard>)], i: usize| -> Option<(f64, Vec<Va>)> {
            let hi = evs.iter().find(|e| e.0 == i && e.2 == vh)?;
            let lo = evs.iter().find(|e| e.0 == i && e.2 == vl && e.4 == hi.4)?;
            Some((
                f64::from(i16::from_le_bytes([lo.3, hi.3])) / 256.0,
                vec![lo.1, hi.1],
            ))
        };
    let acts: BTreeSet<usize> = evs.iter().map(|e| e.0).collect();
    for &i in &acts {
        let Some((v, pcs)) = pair(&evs, i) else {
            continue;
        };
        let own = evs
            .iter()
            .find(|e| e.0 == i && e.2 == vh)
            .map(|e| e.4.clone())
            .unwrap_or_default();
        for cs in h.paths(i, &own) {
            if let Some(c) = cs.iter().find(|c| {
                matches!(c.subj, Subj::Mask(l, _) if Some(l) == f.pad_new) && c.rel == Rel::Ne
            }) {
                if let Subj::Mask(_, m) = c.subj {
                    let bit = m.trailing_zeros() as usize;
                    let name = if f.pad_first_high {
                        BUTTONS.get(7 - bit)
                    } else {
                        BUTTONS.get(bit)
                    };
                    if m.count_ones() == 1
                        && let Some(n) = name
                    {
                        h.fact(
                            "entities.player.jump.button",
                            json!(n),
                            "the button whose new press writes the jump speed",
                            &[c.pc],
                        );
                    }
                }
                h.fact(
                    "entities.player.jump.speed",
                    json!(v),
                    "the 16-bit speed written on a new press while the player stands",
                    &pcs,
                );
                found_jump = true;
            }
        }
    }
    if !found_jump {
        h.open(
            "entities.player.jump.speed",
            None,
            &[vh, vl],
            "no speed written under a new press",
        );
    }
    // Gravity: a constant added to the speed; the limit: a compare with n
    // then n written.
    for c in h.chains16() {
        if !c.sub
            && c.lo == vl
            && c.hi == vh
            && let (Err(a), Err(b)) = (c.add_lo, c.add_hi)
        {
            let g = f64::from(u16::from_le_bytes([a, b])) / 256.0;
            h.fact(
                "entities.player.jump.gravity",
                json!(format!("{g}")),
                "a 16-bit constant added to the vertical speed",
                &[c.pc],
            );
            let r = h.code.routines.at(c.pc);
            for (i, e) in h.events() {
                if let Ev::Write {
                    pc,
                    loc,
                    val,
                    guards,
                    ..
                } = e
                    && *loc == vh
                    && h.code.routines.at(*pc) == r
                    && let Some(n) = val.konst()
                {
                    let cap = h.paths(i, guards).iter().find_map(|cs| {
                        cs.iter()
                            .find(|x| {
                                matches!(x.subj, Subj::Byte(l, 0) if l == vh)
                                    && x.rel == Rel::Ge
                                    && x.k == n
                            })
                            .map(|x| x.pc)
                    });
                    if let Some(cpc) = cap {
                        h.fact(
                            "entities.player.jump.max_fall",
                            json!(n),
                            "the speed is compared with n and set to n",
                            &[cpc, *pc],
                        );
                        break;
                    }
                }
            }
        }
    }
    // Walk and run: a loop counted down by a byte, calling a routine that
    // steps x; the byte's constants, with and without a button.
    let Some(px) = f.px else { return };
    let stepping: BTreeSet<Va> = h
        .events()
        .filter_map(|(i, e)| match e {
            Ev::Step { loc, .. } if *loc == px => Some(h.r.acts[i].start),
            _ => None,
        })
        .collect();
    for lp in h.loops() {
        let Some(m) = lp.by_byte else { continue };
        let calls = h
            .code
            .insns
            .range(lp.head..=lp.back)
            .any(|(_, i)| i.mn == Mn::Jsr && stepping.contains(&i.arg));
        if !calls {
            continue;
        }
        let r = h.code.routines.at(lp.head);
        let mut ws: Vec<(u8, Option<u8>, Va)> = Vec::new();
        for (i, e) in h.events() {
            if let Ev::Write {
                pc,
                loc,
                val,
                guards,
                ..
            } = e
                && *loc == m
                && h.code.routines.at(*pc) == r
                && *pc < lp.head
                && let Some(k) = val.konst()
            {
                let button = h
                    .paths(i, guards)
                    .iter()
                    .flatten()
                    .find_map(|c| match c.subj {
                        Subj::Mask(l, mm) if Some(l) == f.pad && c.rel == Rel::Ne => Some(mm),
                        _ => None,
                    });
                if !ws.iter().any(|w| w.0 == k) {
                    ws.push((k, button, *pc));
                }
            }
        }
        let walk = ws.iter().find(|w| w.1.is_none());
        let run = ws.iter().find(|w| w.1.is_some());
        if let (Some(w), Some(rn)) = (walk, run)
            && ws.len() == 2
        {
            h.fact(
                "entities.player.run.walk_speed",
                json!(w.0),
                "steps per frame: the loop count written without a button",
                &[w.2, lp.back],
            );
            h.fact(
                "entities.player.run.run_speed",
                json!(rn.0),
                "steps per frame: the loop count written under a button",
                &[rn.2, lp.back],
            );
            if let Some(mm) = rn.1 {
                let bit = mm.trailing_zeros() as usize;
                let name = if f.pad_first_high {
                    BUTTONS.get(7 - bit)
                } else {
                    BUTTONS.get(bit)
                };
                if let Some(n) = name {
                    h.fact(
                        "entities.player.run.button",
                        json!(n),
                        "the button that writes the run's count",
                        &[rn.2],
                    );
                }
            }
        } else {
            h.open(
                "entities.player.run",
                Some(lp.head),
                &[m],
                "the step loop's counts aren't one with a button and one without",
            );
        }
    }
}

/// The camera: the byte the NMI writes to the scroll register, and how it
/// follows the player.
fn camera(h: &mut H, f: &mut Found) {
    let nmi = u32::from(h.code.word(0xFFFA));
    let mut cam = None;
    for (i, e) in h.events() {
        if let Ev::Write { loc, val, pc, .. } = e
            && loc.addr == 0x2005
            && h.r.acts[i].start == nmi
            && let Some((Some(c), None, 0)) = val.lin()
        {
            cam = Some((c, *pc));
            break;
        }
    }
    let Some((c, pc)) = cam else {
        h.open(
            "cameras.main",
            None,
            &[],
            "no byte is written to the scroll register",
        );
        return;
    };
    f.camera = Some(c);
    h.fact(
        "cameras.main.bytes.x",
        json!(hx(&c)),
        "the byte the NMI writes to the scroll register",
        &[pc],
    );
    let Some(px) = f.px else { return };
    let mut lead = None;
    let mut clamp = Vec::new();
    for (_, e) in h.events() {
        if let Ev::Write { loc, val, pc, .. } = e
            && *loc == c
        {
            match val.lin() {
                Some((Some(p), None, off)) if p == px => lead = Some((-off, *pc)),
                Some((None, None, _)) => clamp.push(*pc),
                _ => {}
            }
        }
    }
    match lead {
        Some((k, pc)) => {
            h.fact(
                "cameras.main.lead",
                json!([k, 0]),
                "the camera's x is the player's x minus a constant",
                &[pc],
            );
            h.fact(
                "cameras.main.follow",
                json!("player"),
                "the camera's x is written from the player's",
                &[pc],
            );
            h.fact(
                "cameras.main.axis",
                json!("x"),
                "only the horizontal scroll follows",
                &[pc],
            );
            if !clamp.is_empty() {
                h.fact(
                    "cameras.main.clamp_to_level",
                    json!(true),
                    "constants written to the camera's x at the level's ends",
                    &clamp,
                );
            }
        }
        None => h.open(
            "cameras.main.lead",
            None,
            &[c, px],
            "the camera's x isn't the player's x minus a constant",
        ),
    }
}

/// A constant written beside a state: the place, the value, the
/// instruction.
type Entered = (Loc, u8, Va);

/// Whether an activation runs only before the frame loop: every caller
/// chain goes back to the reset code, none through the loop or the NMI.
fn init_only(h: &H, act: usize) -> bool {
    let reset = u32::from(h.code.word(0xFFFC));
    let mut seen = BTreeSet::new();
    let mut todo = vec![act];
    let mut roots = BTreeSet::new();
    while let Some(a) = todo.pop() {
        if !seen.insert(a) {
            continue;
        }
        if h.r.acts[a].callers.is_empty() || h.r.loops.contains(&h.r.acts[a].start) {
            roots.insert(h.r.acts[a].start);
        } else {
            todo.extend(h.r.acts[a].callers.iter().map(|c| c.0));
        }
    }
    roots.len() == 1 && roots.contains(&reset) && !h.r.loops.contains(&reset)
}

/// State machines: each state byte's states, and its transitions with
/// their guards, timers and counter tests.
fn machines(h: &mut H, f: &mut Found) {
    let states = f.states.clone();
    let mut entered_with: BTreeMap<(Loc, u8), Vec<Entered>> = BTreeMap::new();
    for (s, values) in &states {
        let mut m = Machine {
            byte: hx(s),
            name: byte_name(s),
            states: Vec::new(),
            transitions: Vec::new(),
            initial: None,
            path: String::new(),
        };
        for v in values {
            m.states.push(State {
                value: *v,
                name: state_name(*v),
            });
        }
        // Each constant write to the byte, with every path's conditions.
        let mut ws = Vec::new();
        for (i, e) in h.events() {
            if let Ev::Write {
                loc,
                val,
                guards,
                pc,
                ..
            } = e
                && loc == s
                && let Some(to) = val.konst()
            {
                ws.push((i, *pc, to, guards.clone()));
            }
        }
        let mut inits: BTreeSet<u8> = BTreeSet::new();
        // The game state's values the reset code writes: a write of one of
        // them later starts the level again.
        let game_init: BTreeSet<u8> = f.game.map_or_else(BTreeSet::new, |g| {
            h.events()
                .filter_map(|(i, e)| match e {
                    Ev::Write { loc, val, .. } if *loc == g && init_only(h, i) => val.konst(),
                    _ => None,
                })
                .collect()
        });
        for (i, pc, to, own) in ws {
            // Power-on: a write the reset code makes before the frame loop
            // is the machine's start, not a transition.
            if init_only(h, i) {
                inits.insert(to);
                continue;
            }
            // Another machine's state written where the game's state is
            // written too is the level's start (spawn), not a transition.
            let starts_level = Some(*s) != f.game
                && f.game.is_some_and(|g| {
                    h.r.acts[i].events.iter().any(|e| matches!(e, Ev::Write { loc, val, .. } if *loc == g && val.konst().is_some_and(|v| game_init.contains(&v))))
                });
            if starts_level {
                continue;
            }
            // What else this activation writes as constants: a timer set on
            // entry, a speed.
            for e in &h.r.acts[i].events {
                if let Ev::Write {
                    loc,
                    val,
                    pc: p2,
                    guards: g2,
                    ..
                } = e
                    && loc != s
                    && *g2 == own
                    && let Some(k) = val.konst()
                {
                    let e = entered_with.entry((*s, to)).or_default();
                    if !e.iter().any(|x| x.0 == *loc && x.1 == k) {
                        e.push((*loc, k, *p2));
                    }
                }
            }
            for cs in h.paths(i, &own) {
                let mut from: BTreeSet<u8> = values.clone();
                let mut guards = Vec::new();
                for c in &cs {
                    if let Some((l, k)) = c.equals()
                        && l == *s
                    {
                        from.retain(|&v| v == k);
                        continue;
                    }
                    if let Some((l, k)) = c.differs()
                        && l == *s
                    {
                        from.remove(&k);
                        continue;
                    }
                    guards.push(c.text());
                }
                guards.sort();
                guards.dedup();
                for fr in from.iter().filter(|&&v| v != to || values.len() == 1) {
                    let t = Transition {
                        from: state_name(*fr),
                        to: state_name(to),
                        at: crate::codemap::va_hex(pc),
                        routine: h.rname(pc),
                        guards: guards.clone(),
                        after: None,
                        counter: None,
                    };
                    if !m
                        .transitions
                        .iter()
                        .any(|x| x.from == t.from && x.to == t.to && x.guards == t.guards)
                    {
                        m.transitions.push(t);
                    }
                }
                let _ = &cs;
            }
        }
        m.transitions
            .sort_by(|a, b| (&a.from, &a.to, &a.guards).cmp(&(&b.from, &b.to, &b.guards)));
        if inits.len() == 1
            && let Some(v) = inits.iter().next()
        {
            m.initial = Some(state_name(*v));
        }
        // The machine explains every test of its byte by value.
        for (_, e) in h.events() {
            if let Ev::Branch { pc, op, test, .. } = e
                && let Some(c) = cond::of(&Guard {
                    pc: *pc,
                    op: *op,
                    taken: true,
                    test: test.clone(),
                })
                && c.byte() == Some(*s)
            {
                h.cited.insert(*pc);
            }
        }
        h.out.machines.push(m);
    }
    // Timers: a byte set on entry to a state, counted down while in it; a
    // transition out on its reaching 0 is `after` that many frames.
    for mi in 0..h.out.machines.len() {
        let s = *states.keys().nth(mi).unwrap_or(&Loc::at(0));
        let mut ts = std::mem::take(&mut h.out.machines[mi].transitions);
        for t in &mut ts {
            let from: u8 = t.from[1..].parse().unwrap_or(0);
            for (timer, k, wpc) in entered_with.get(&(s, from)).cloned().unwrap_or_default() {
                if !f.timers.contains(&timer) {
                    continue;
                }
                let name = format!("{} == 0", hx(&timer));
                let name2 = name.clone();
                if t.guards.iter().any(|g| *g == name || *g == name2) {
                    t.after = Some(u32::from(k));
                    h.cited.insert(wpc);
                    if let Some(prev) = h.code.insns.range(..wpc).next_back() {
                        h.cited.insert(*prev.0);
                    }
                    t.guards.retain(|g| *g != name && *g != name2);
                    // The timer's other test (not yet 0) is the stay.
                    t.guards.retain(|g| *g != format!("{} != 0", hx(&timer)));
                }
            }
            // A counter tested before it is spent.
            for c in &f.counters {
                let ne = format!("{} != 0", hx(c));
                let eq = format!("{} == 0", hx(c));
                if t.guards.contains(&ne) {
                    t.counter = Some(
                        json!({"counter": format!("c_{:02X}", c.addr), "cmp": ">=", "value": 1}),
                    );
                } else if t.guards.contains(&eq) {
                    t.counter = Some(
                        json!({"counter": format!("c_{:02X}", c.addr), "cmp": "==", "value": 0}),
                    );
                }
            }
        }
        h.out.machines[mi].transitions = ts;
    }
    // Facts: the game state's timed transitions; the slot state's.
    let machines = h.out.machines.clone();
    for (mi, m) in machines.iter().enumerate() {
        let s = *states.keys().nth(mi).unwrap_or(&Loc::at(0));
        let base = if Some(s) == f.game {
            format!("rules.{}", m.name)
        } else if let Some(g) = f
            .groups
            .iter()
            .find(|g| s.idx.is_some() && g.2.contains(&s.addr))
        {
            f.gstate = Some(s);
            format!("entities.{}.machine", g.0)
        } else {
            format!("machines.{}", m.name)
        };
        h.out.machines[mi].path = base.clone();
        if let Some(init) = &m.initial {
            h.fact(
                format!("{base}.initial"),
                json!(init),
                "the state the reset code writes before the frame loop",
                &[],
            );
            if Some(s) == f.game && f.px.is_some() {
                h.fact(
                    "entities.player.machine.initial",
                    json!(init),
                    "the player's states are the game's",
                    &[],
                );
            }
        }
        for t in &m.transitions {
            if let Some(a) = t.after {
                let pc = crate::codemap::va_parse(&t.at).unwrap_or(0);
                h.fact(format!("{base}.states.{}.transition[{}].after", t.from, t.to), json!(a), "a timer set on entering the state, counted down in it; the transition runs when it reaches 0", &[pc]);
                if let Some(c) = &t.counter {
                    h.fact(
                        format!("{base}.states.{}.transition[{}].if.value", t.from, t.to),
                        c["value"].clone(),
                        "the counter tested before the transition",
                        &[pc],
                    );
                    h.fact(
                        format!("{base}.states.{}.transition[{}].if.counter", t.from, t.to),
                        c["counter"].clone(),
                        "the counter tested before the transition",
                        &[pc],
                    );
                    h.fact(
                        format!("{base}.states.{}.transition[{}].if.cmp", t.from, t.to),
                        c["cmp"].clone(),
                        "the counter tested before the transition",
                        &[pc],
                    );
                }
            }
        }
    }
    // The game's start value: the state written with the player's start
    // place, the death state: written with a speed to the player.
    if let Some(g) = f.game {
        for ((s, v), with) in &entered_with {
            if *s != g {
                continue;
            }
            if let Some((hi, lo)) = f.vy {
                let a = with.iter().find(|w| w.0 == hi);
                let b = with.iter().find(|w| w.0 == lo);
                if let (Some(a), Some(b)) = (a, b)
                    && (a.1, b.1) != (0, 0)
                {
                    let sp = f64::from(i16::from_le_bytes([b.1, a.1])) / 256.0;
                    f.dying = Some(*v);
                    h.fact(
                        format!(
                            "entities.player.machine.states.{}.enter_speed",
                            state_name(*v)
                        ),
                        json!([0, sp]),
                        "the vertical speed written with the game state",
                        &[b.2, a.2],
                    );
                }
            }
            if let (Some(px), Some(py)) = (f.px, f.py)
                && with.iter().any(|w| w.0 == px)
                && with.iter().any(|w| w.0 == py)
            {
                f.initial = Some(*v);
                h.fact(
                    format!("rules.{}.initial", byte_name(&g)),
                    json!(state_name(*v)),
                    "the state written with the player's start place",
                    &[],
                );
            }
        }
    }
}

/// Touching a slot: the overlap tests, the stomp and the bounce.
fn contact(h: &mut H, f: &mut Found) {
    let (Some(px), Some(py), Some(gs)) = (f.px, f.py, f.gstate) else {
        return;
    };
    let Some(g) = f.groups.iter().find(|g| g.2.contains(&gs.addr)).cloned() else {
        return;
    };
    // The slot's x: an array stepped by 1, not when another wraps (that is
    // its high byte).
    let mut stepped: BTreeSet<Va> = BTreeSet::new();
    let mut high: BTreeSet<Va> = BTreeSet::new();
    for (_, e) in h.events() {
        if let Ev::Step {
            loc, guards, by, ..
        } = e
            && loc.idx.is_some()
            && g.2.contains(&loc.addr)
            && !f.timers.contains(loc)
        {
            stepped.insert(loc.addr);
            let wraps = wraps(*loc, *by, guards);
            if wraps {
                high.insert(loc.addr);
            }
        }
    }
    let gx = stepped.difference(&high).next().copied();
    f.gx = gx;
    let Some(gx) = gx else {
        h.open(
            format!("entities.{}.patrol", g.0),
            None,
            &[],
            "no slot array is stepped",
        );
        return;
    };
    h.fact(
        format!("entities.{}.bytes.x", g.0),
        json!(format!("${gx:02X},x")),
        "the slot array stepped by one",
        &[],
    );
    h.fact(
        format!("entities.{}.count", g.0),
        json!(g.1),
        "the loop count over the slots",
        &[],
    );
    // The patrol: the frame mask on the stepping, and the step.
    let gloc = Loc {
        addr: gx,
        idx: Some(Reg::X),
    };
    let mut steppers = Vec::new();
    for (i, e) in h.events() {
        if let Ev::Step {
            loc,
            guards,
            pc,
            by,
        } = e
            && *loc == gloc
        {
            steppers.push((i, *pc, *by, guards.clone()));
        }
    }
    // A slot kind's own routine (an entry of a jump table on a slot
    // array) steps only that kind: its facts go under the kind.
    let kinds: BTreeSet<String> = steppers
        .iter()
        .map(|(i, ..)| {
            let r = h.rname(h.r.acts[*i].start);
            let mut name = g.0.clone();
            for d in &h.out.dispatch {
                let on_slots = d.kind == "jump table"
                    && g.2.iter().any(|a| d.subject == format!("${a:02X},x"));
                if let Some((k, _)) = d.cases.iter().find(|(_, rs)| rs.contains(&r))
                    && on_slots
                {
                    name = format!("{}_k{}", g.0, k.parse::<u32>().unwrap_or(0));
                }
            }
            name
        })
        .collect();
    let ent = if kinds.len() == 1 {
        kinds.iter().next().cloned().unwrap_or_default()
    } else {
        g.0.clone()
    };
    if kinds.len() > 1 {
        h.open(
            format!("entities.{}.patrol", g.0),
            None,
            &[gloc],
            "more than one slot kind steps the slots' x",
        );
    }
    let mut period = None;
    let mut state_ok: BTreeSet<u8> = BTreeSet::new();
    for (i, pc, by, own) in &steppers {
        for cs in h.paths(*i, own) {
            for c in &cs {
                if let Subj::Mask(l, m) = c.subj
                    && Some(l) == f.frame
                    && c.rel == Rel::Eq
                    && (u16::from(m) + 1).is_power_of_two()
                {
                    period = Some((u16::from(m) + 1, c.pc));
                }
                if let Some((l, k)) = c.equals()
                    && l == gs
                {
                    state_ok.insert(k);
                }
            }
        }
        h.fact(
            format!("entities.{ent}.patrol.step"),
            json!(by.unsigned_abs()),
            "the slot's x is stepped by this much",
            &[*pc],
        );
    }
    if let Some((p, pc)) = period {
        h.fact(
            format!("entities.{ent}.patrol.period"),
            json!(p),
            "the stepping runs when the frame counter's low bits are 0",
            &[pc],
        );
    }
    let values = f.states.get(&gs).cloned().unwrap_or_default();
    if !state_ok.is_empty() {
        for v in values.iter().filter(|v| !state_ok.contains(v)) {
            h.fact(
                format!(
                    "entities.{ent}.machine.states.{}.set.patrol.step",
                    state_name(*v)
                ),
                json!(0),
                "the stepping runs only in other states",
                &[],
            );
            if let Some((p, _)) = period {
                h.fact(
                    format!(
                        "entities.{ent}.machine.states.{}.set.patrol.period",
                        state_name(*v)
                    ),
                    json!(p),
                    "the period is the frame mask's, whatever the state",
                    &[],
                );
            }
        }
    }
    // The touch: the loop over the slots whose writes include a new state
    // and a speed for the player; its skips are the overlap.
    let Some((vh, vl)) = f.vy else { return };
    let loops = h.loops();
    let Some(lp) = loops.iter().find(|lp| {
        lp.reg == Some(Reg::X)
            && h.code
                .insns
                .range(lp.head..=lp.back)
                .any(|(_, i)| i.mn == Mn::Sta && i.arg == gs.addr && i.mode == Mode::ZpX)
            && h.code
                .insns
                .range(lp.head..=lp.back)
                .any(|(_, i)| i.mn == Mn::Sta && i.arg == vh.addr)
    }) else {
        h.open(
            format!("entities.{}.contact", g.0),
            None,
            &[gs],
            "no loop over the slots writes both a slot state and the player's speed",
        );
        return;
    };
    let mut keep = BTreeMap::new();
    let mut stomp_write = None;
    for (i, e) in h.events() {
        if let Ev::Branch { pc, op, test, .. } = e
            && *pc >= lp.head
            && *pc <= lp.back
        {
            let Some(ins) = h.code.insns.get(pc) else {
                continue;
            };
            // The way that doesn't skip to the next slot.
            let taken_skips = ins.arg == lp.next;
            let fall_skips = h
                .code
                .insns
                .get(&ins.next())
                .is_some_and(|n| n.mn == Mn::Jmp && n.arg == lp.next);
            if !taken_skips && !fall_skips {
                continue;
            }
            let g2 = Guard {
                pc: *pc,
                op: *op,
                taken: !taken_skips,
                test: test.clone(),
            };
            if let Some(c) = cond::of(&g2) {
                keep.insert((c.text(), *pc), c);
            }
        }
        if let Ev::Write {
            loc,
            val,
            pc,
            guards,
            ..
        } = e
            && *loc == gs
            && *pc >= lp.head
            && *pc <= lp.back
            && let Some(v) = val.konst()
        {
            stomp_write = Some((i, *pc, v, guards.clone()));
        }
    }
    let Some((si, spc, sv, sg)) = stomp_write else {
        return;
    };
    // Touched any other way: the touch routine calls one that writes the
    // death state.
    if let (Some(g0), Some(d)) = (f.game, f.dying) {
        let kills = h.r.acts[si].events.iter().any(|e| match e {
            Ev::Call { act, .. } if *act != usize::MAX => h.r.acts[*act].events.iter().any(
                |x| matches!(x, Ev::Write { loc, val, .. } if *loc == g0 && val.konst() == Some(d)),
            ),
            _ => false,
        });
        if kills {
            h.fact(
                format!("entities.{}.role", g.0),
                json!("enemy"),
                "touching one any other way calls the routine that writes the death state",
                &[spc],
            );
            h.fact(
                format!("entities.{}.contact.otherwise", g.0),
                json!("kill-player"),
                "touching one any other way calls the routine that writes the death state",
                &[spc],
            );
        }
    }
    // The bounce: the speed written beside the slot's new state.
    let mut hi = None;
    let mut lo = None;
    for e in &h.r.acts[si].events {
        if let Ev::Write {
            loc,
            val,
            pc,
            guards,
            ..
        } = e
            && guards == &sg
            && let Some(k) = val.konst()
        {
            if *loc == vh {
                hi = Some((k, *pc));
            }
            if *loc == vl {
                lo = Some((k, *pc));
            }
        }
    }
    if let (Some(a), Some(b)) = (hi, lo) {
        let sp = f64::from(i16::from_le_bytes([b.0, a.0])) / 256.0;
        h.fact(
            format!("entities.{}.contact.bounce", g.0),
            json!(sp),
            "the player's speed written when a slot's state changes on a touch",
            &[b.1, a.1],
        );
    }
    h.fact(
        format!("entities.{}.contact.keep_on_stomp", g.0),
        json!(sv != 0),
        "the slot's state after a stomp isn't 0",
        &[spc],
    );
    for v in values
        .iter()
        .filter(|v| !keep.values().any(|c| c.equals() == Some((gs, **v))))
    {
        h.fact(
            format!(
                "entities.{}.machine.states.{}.set.contact.stomp_depth",
                g.0,
                state_name(*v)
            ),
            json!(0),
            "the touch loop skips slots in this state",
            &[],
        );
        h.fact(
            format!(
                "entities.{}.machine.states.{}.set.contact.otherwise",
                g.0,
                state_name(*v)
            ),
            json!("nothing"),
            "the touch loop skips slots in this state",
            &[],
        );
    }
    // The overlap: py in [a, b), px - sx in [c, d].
    let mut y_lo = None;
    let mut y_hi = None;
    let mut d_lo = None;
    let mut d_hi = None;
    let mut pcs = Vec::new();
    for c in keep.values() {
        match (&c.subj, c.rel) {
            (Subj::Byte(l, 0), Rel::Ge) if *l == py => y_lo = Some(i32::from(c.k)),
            (Subj::Byte(l, 0), Rel::Lt) if *l == py => y_hi = Some(i32::from(c.k) - 1),
            (Subj::Diff(a, b, 0), Rel::Lt) if *a == px && b.addr == gx => {
                d_hi = Some(i32::from(c.k) - 1)
            }
            (Subj::Diff(a, b, 0), Rel::Ge) if *a == px && b.addr == gx => {
                d_lo = Some(i32::from(c.k) - 256)
            }
            _ => continue,
        }
        pcs.push(c.pc);
    }
    let sy = f.group_y.or_else(|| group_y(h, f));
    f.group_y = sy;
    let Some((x, w, y, hh)) = f.hitbox else {
        return;
    };
    if let (Some(dl), Some(dh)) = (d_lo, d_hi) {
        let bx = dl + x + w - 1;
        let bw = dh - bx + x + 1;
        h.fact(
            format!("entities.{}.hitbox.x", g.0),
            json!(bx),
            "solved from the touch's x overlap test and the player's box",
            &pcs,
        );
        h.fact(
            format!("entities.{}.hitbox.w", g.0),
            json!(bw),
            "solved from the touch's x overlap test and the player's box",
            &pcs,
        );
    } else {
        h.open(
            format!("entities.{}.hitbox.x", g.0),
            Some(lp.head),
            &[px, gloc],
            "no x overlap test found in the touch loop",
        );
    }
    match (y_lo, y_hi, sy) {
        (Some(yl), Some(yh), Some(sy)) => {
            let by = yl - sy + y + hh - 1;
            let bh = yh - by - sy + y + 1;
            h.fact(
                format!("entities.{}.hitbox.y", g.0),
                json!(by),
                "solved from the touch's y overlap test, the slot's y and the player's box",
                &pcs,
            );
            h.fact(
                format!("entities.{}.hitbox.h", g.0),
                json!(bh),
                "solved from the touch's y overlap test, the slot's y and the player's box",
                &pcs,
            );
            // The stomp: the player's y below a constant on the stomp path.
            for cs in h.paths(si, &sg) {
                if let Some(c) = cs
                    .iter()
                    .find(|c| matches!(c.subj, Subj::Byte(l, 0) if l == py) && c.rel == Rel::Lt)
                {
                    let depth = i32::from(c.k) - 1 + y + hh - 1 - (sy + by) + 1;
                    h.fact(format!("entities.{}.contact.stomp_depth", g.0), json!(depth), "the rows of the slot's top the player's bottom may be in for a stomp, from the stomp's y test", &[c.pc]);
                    break;
                }
            }
        }
        _ => h.open(
            format!("entities.{}.hitbox.y", g.0),
            Some(lp.head),
            &[py],
            "the touch's y test or the slot's y isn't known",
        ),
    }
}

/// A slot's y when the code holds it as a constant: the sprite row it is
/// drawn at, less the offset the player's sprite row has from its y.
fn group_y(h: &mut H, f: &mut Found) -> Option<i32> {
    let py = f.py?;
    let gs = f.gstate?;
    let mut player_off = None;
    let mut consts = BTreeSet::new();
    let mut pc0 = None;
    for (i, e) in h.events() {
        let Ev::Write {
            loc,
            val,
            guards,
            pc,
            ..
        } = e
        else {
            continue;
        };
        if loc.addr != f.oam || loc.idx.is_none() {
            continue;
        }
        match val.lin() {
            Some((Some(p), None, off)) if p == py => {
                player_off = Some(player_off.map_or(off, |o: i16| o.min(off)));
            }
            Some((None, None, k)) => {
                let slot = h
                    .paths(i, guards)
                    .iter()
                    .any(|cs| cs.iter().any(|c| c.places().contains(&gs)));
                if slot {
                    consts.insert(k);
                    pc0 = Some(*pc);
                }
            }
            _ => {}
        }
    }
    let off = player_off?;
    let top = *consts.iter().next()?;
    let y = i32::from(top) - i32::from(off);
    if let Some(g) = f.groups.iter().find(|g| g.2.contains(&gs.addr)).cloned() {
        h.fact(
            format!("levels.main.spawn[{}].y", g.0),
            json!(y),
            "the slot's sprite row, a constant, less the player's sprite offset",
            &pc0.into_iter().collect::<Vec<_>>(),
        );
    }
    Some(y)
}

/// The level: where the player starts, the pit, the goal, the slots'
/// spawn table.
fn level(h: &mut H, f: &mut Found) {
    let (Some(px), Some(py), Some(g)) = (f.px, f.py, f.game) else {
        return;
    };
    let mut ws = Vec::new();
    for (i, e) in h.events() {
        if let Ev::Write {
            loc,
            val,
            guards,
            pc,
            ..
        } = e
            && *loc == g
            && let Some(v) = val.konst()
        {
            ws.push((i, *pc, v, guards.clone()));
        }
    }
    for (i, pc, v, own) in ws {
        // Start: the activation writing the initial state writes the
        // player's place.
        if Some(v) == f.initial {
            for e in &h.r.acts[i].events {
                if let Ev::Write {
                    loc, val, pc: p2, ..
                } = e
                    && let Some(k) = val.konst()
                {
                    if *loc == px {
                        h.fact(
                            "levels.main.spawn[player].entity",
                            json!("player"),
                            "the start state's routine places the player",
                            &[*p2],
                        );
                        h.fact(
                            "levels.main.spawn[player].x",
                            json!(k),
                            "the player's x written with the start state",
                            &[*p2],
                        );
                    }
                    if *loc == py {
                        h.fact(
                            "levels.main.spawn[player].y",
                            json!(k),
                            "the player's y written with the start state",
                            &[*p2],
                        );
                    }
                }
            }
        }
        for cs in h.paths(i, &own) {
            let slot = cs
                .iter()
                .any(|c| c.places().iter().any(|l| l.idx.is_some()));
            if slot {
                continue;
            }
            // The pit: the death state, written when y is at or below k.
            if Some(v) == f.dying
                && let Some(c) = cs
                    .iter()
                    .find(|c| matches!(c.subj, Subj::Byte(l, 0) if l == py) && c.rel == Rel::Ge)
            {
                h.fact(
                    "levels.main.kill_below",
                    json!(c.k),
                    "the death state is written when the player's y is at or past this",
                    &[c.pc, pc],
                );
            }
            // The goal: another state, written when x is past a 16-bit
            // constant.
            if Some(v) != f.dying && Some(v) != f.initial {
                let hi = cs.iter().find(|c| {
                    Some(c.subj.clone()) == f.px_hi.map(|l| Subj::Byte(l, 0))
                        && c.rel == Rel::Ne
                        && c.k == 0
                });
                let lo = cs
                    .iter()
                    .find(|c| matches!(c.subj, Subj::Byte(l, 0) if l == px) && c.rel == Rel::Ge);
                if let (Some(_), Some(lo)) = (hi, lo) {
                    // The high byte is 0 or 1 when the level is at most two
                    // screens of 256 wide.
                    let w = h.inp.level_width_px;
                    if w > 0 && w <= 512 {
                        h.fact("levels.main.goal.x", json!(256 + u32::from(lo.k)), "a state written when the player's 16-bit x is at or past this; the level is two screens wide, so its high byte is 1", &[lo.pc, pc]);
                    } else {
                        h.open("levels.main.goal.x", Some(pc), &[px], "the high byte's test is != 0 and the level's width doesn't bound it to 1");
                    }
                }
            }
        }
    }
    // The slots' spawn table: ROM arrays loaded into the slot's x.
    if let Some(gx) = f.gx {
        let g0 = f
            .groups
            .iter()
            .find(|g| g.2.contains(&gx))
            .map(|g| g.0.clone())
            .unwrap_or_default();
        let mut cols = BTreeMap::new();
        for (_, e) in h.events() {
            if let Ev::Write { loc, val, pc, .. } = e
                && loc.addr == gx
                && let Some(src) = val.from.or_else(|| kind_places(val).into_iter().next())
                && src.rom()
                && let Some(t) = h.table_of(src.addr)
            {
                cols.insert("x", (t.asset.clone(), t.column.clone(), *pc));
            }
            if let Ev::Branch {
                test: super::absint::Flags::Cmp { lhs, rhs },
                pc,
                ..
            } = e
                && lhs.from.is_some_and(|l| l.addr == gx)
                && let Some(src) = rhs.from
                && src.rom()
                && let Some(t) = h.table_of(src.addr)
                && !cols
                    .values()
                    .any(|c: &(String, String, Va)| c.1 == t.column)
            {
                cols.insert("patrol_end", (t.asset.clone(), t.column.clone(), *pc));
            }
        }
        for (k, (asset, col, pc)) in cols {
            h.fact(
                format!("levels.main.spawn[{g0}].entity"),
                json!(g0),
                "the slots are spawned from a table",
                &[pc],
            );
            h.fact(
                format!("levels.main.spawn[{g0}].table"),
                json!(asset),
                "the asset table the slots' x is loaded from",
                &[pc],
            );
            h.fact(
                format!("levels.main.spawn[{g0}].columns.{k}"),
                json!(col),
                "the table column the slots' x is loaded from or compared with",
                &[pc],
            );
        }
    }
}

/// Counters: start values, floors, and what changes them.
fn counters(h: &mut H, f: &mut Found) {
    let cs = f.counters.clone();
    // The new game: the activation writing constants to the most counters.
    let mut best: Option<(usize, usize)> = None;
    for (i, a) in h.r.acts.iter().enumerate() {
        let n = a.events.iter().filter(|e| matches!(e, Ev::Write { loc, val, .. } if cs.contains(loc) && val.konst().is_some())).count();
        if n >= 2 && best.is_none_or(|b| n > b.1) {
            best = Some((i, n));
        }
    }
    for c in &cs {
        let name = format!("c_{:02X}", c.addr);
        h.fact(
            format!("counters.{name}.bytes"),
            json!(hx(c)),
            "a byte changed only by 1 at a time and set by constants",
            &[],
        );
        if let Some((i, _)) = best {
            for e in &h.r.acts[i].events {
                if let Ev::Write { loc, val, pc, .. } = e
                    && loc == c
                    && let Some(k) = val.konst()
                {
                    h.fact(
                        format!("counters.{name}.start"),
                        json!(k),
                        "the constant the new game writes",
                        &[*pc],
                    );
                }
            }
        }
        // A floor: every decrement is guarded by != 0.
        let mut decs = Vec::new();
        for (i, e) in h.events() {
            if let Ev::Step {
                loc,
                by: -1,
                guards,
                pc,
            } = e
                && loc == c
            {
                decs.push((i, *pc, guards.clone()));
            }
        }
        if !decs.is_empty() {
            let floor = decs.iter().all(|(i, _, g)| {
                h.paths(*i, g)
                    .iter()
                    .all(|cs| cs.iter().any(|x| x.differs() == Some((*c, 0))))
            });
            if floor {
                let pcs: Vec<Va> = decs.iter().map(|d| d.1).collect();
                h.fact(
                    format!("counters.{name}.min"),
                    json!(0),
                    "every decrement is skipped at 0",
                    &pcs,
                );
            }
        }
    }
}

/// The model's non-numeric parts the code and the asset map settle: the
/// level's asset slots and properties, the player's role, states in which
/// nothing moves.
fn shape(h: &mut H, f: &Found) {
    if f.px.is_some() {
        h.fact(
            "entities.player.role",
            json!("player"),
            "the object the controller's buttons move",
            &[],
        );
    }
    if f.hitbox.is_some() {
        h.fact(
            "entities.player.tile_collision",
            json!(true),
            "the level's cells are looked up around it",
            &[],
        );
    }
    let grid = h
        .inp
        .tables
        .iter()
        .find(|t| t.asset_kind == "level-grid")
        .map(|t| t.asset.clone());
    let tiles = h
        .inp
        .tables
        .iter()
        .find(|t| t.asset_kind == "metatiles")
        .map(|t| t.asset.clone());
    if let Some(g) = grid {
        h.fact(
            "levels.main.grid",
            json!(g),
            "the asset map's level grid",
            &[],
        );
    }
    if let Some(t) = tiles {
        h.fact(
            "levels.main.tileset",
            json!(t),
            "the asset map's metatiles",
            &[],
        );
    }
    // The properties the code reads: the one tested for a bit after a
    // probe (solid), the one compared with constants (kind).
    let mut solid = None;
    let mut kind = None;
    for (_, e) in h.events() {
        if let Ev::Branch { test, pc, .. } = e {
            let l = match test {
                super::absint::Flags::Carry { src, .. } => src.from,
                super::absint::Flags::Res {
                    src, mask: None, ..
                } => *src,
                _ => None,
            };
            if let Some(l) = l
                && let Some(t) = h.table_of(l.addr)
                && t.asset_kind == "metatiles"
                && t.column != t.asset
            {
                solid.get_or_insert((t.column.clone(), *pc));
            }
            if let super::absint::Flags::Cmp { lhs, rhs } = test
                && rhs.konst().is_some()
                && let Some(l) = lhs.from
                && let Some(t) = h.table_of(l.addr)
                && t.asset_kind == "metatiles"
                && t.column != t.asset
            {
                kind.get_or_insert((t.column.clone(), *pc));
            }
        }
    }
    if let Some((c, pc)) = solid {
        h.fact(
            "levels.main.solid_property",
            json!(c),
            "the metatile property tested for a bit after each probe",
            &[pc],
        );
    }
    if let Some((c, pc)) = kind {
        h.fact(
            "levels.main.kind_property",
            json!(c),
            "the metatile property compared with constants",
            &[pc],
        );
    }
    // Air control: the horizontal steps aren't under a test of a byte the
    // landing writes.
    if let Some(px) = f.px {
        let landing: BTreeSet<Loc> = h
            .events()
            .filter_map(|(_, e)| match e {
                Ev::Write { loc, val, .. } if f.player.contains(loc) && val.konst() == Some(1) => {
                    Some(*loc)
                }
                _ => None,
            })
            .collect();
        let mut any = false;
        let mut grounded = false;
        for (i, e) in h.events() {
            if let Ev::Step { loc, guards, .. } = e
                && *loc == px
            {
                for cs in h.paths(i, guards) {
                    if cs.iter().any(|c| is_pad_cond(f, c)) {
                        any = true;
                        grounded |= cs
                            .iter()
                            .any(|c| c.byte().is_some_and(|l| landing.contains(&l)));
                    }
                }
            }
        }
        if any {
            h.fact(
                "entities.player.run.air_control",
                json!(!grounded),
                "the steps under the buttons don't test whether the player stands",
                &[],
            );
        }
    }
    // The death state: what the player does in it.
    if let (Some(g), Some(d)) = (f.game, f.dying) {
        let st = format!("entities.player.machine.states.{}", state_name(d));
        let el: BTreeMap<String, String> = h
            .out
            .routines
            .iter()
            .map(|r| (r.name.clone(), r.element.clone()))
            .collect();
        if let Some(disp) = h
            .out
            .dispatch
            .iter()
            .find(|x| x.kind == "compare chain" && x.subject == hx(&g))
            .cloned()
        {
            let rs: Vec<String> = disp
                .cases
                .iter()
                .filter(|(k, _)| k.split(',').any(|x| x == d.to_string()))
                .flat_map(|(_, r)| r.clone())
                .collect();
            let calls: BTreeMap<String, Vec<String>> = h
                .out
                .routines
                .iter()
                .map(|r| (r.name.clone(), r.calls.clone()))
                .collect();
            let mut seen = BTreeSet::new();
            let mut todo = rs.clone();
            while let Some(x) = todo.pop() {
                if seen.insert(x.clone()) {
                    todo.extend(calls.get(&x).cloned().unwrap_or_default());
                }
            }
            // A cell looked up around the player on a path in this state.
            // Settled only when every probe path says which state it is in.
            let mut probes = Some(false);
            for &a in &f.probe_acts {
                for cs in h.paths(a, &BTreeSet::new()) {
                    let here = cs.iter().any(|c| c.equals() == Some((g, d)));
                    let elsewhere = cs.iter().any(|c| {
                        c.differs() == Some((g, d))
                            || c.equals().is_some_and(|(l, k)| l == g && k != d)
                    });
                    if here {
                        probes = Some(true);
                    } else if !elsewhere && probes == Some(false) {
                        probes = None;
                    }
                }
            }
            let _ = &el;
            let steps = seen.iter().any(|x| {
                h.r.acts.iter().any(|a| {
                    h.rname(a.start) == *x
                        && a.events
                            .iter()
                            .any(|e| matches!(e, Ev::Step { loc, .. } if Some(*loc) == f.px))
                })
            });
            if !rs.is_empty() {
                match probes {
                    Some(p) => h.fact(format!("{st}.set.tile_collision"), json!(p), "whether a cell is looked up around the player on a path in the death state", &[]),
                    None => h.open(format!("{st}.set.tile_collision"), None, &[g], "a probe runs on a path that doesn't say the game's state"),
                }
                h.fact(
                    format!("{st}.set.run.off"),
                    json!(!steps),
                    "the death state's routines don't step the player sideways",
                    &[],
                );
            }
        }
        // Leaving the ground: the death's routine writes the standing flag 0.
        let leaves = h.r.acts.iter().find_map(|a| {
            if !a.events.iter().any(
                |e| matches!(e, Ev::Write { loc, val, .. } if *loc == g && val.konst() == Some(d)),
            ) {
                return None;
            }
            a.events.iter().find_map(|e| match e {
                Ev::Write { loc, val, pc, .. }
                    if f.player.contains(loc)
                        && Some(*loc) != f.px
                        && Some(*loc) != f.py
                        && val.konst() == Some(0)
                        && !f.states.contains_key(loc)
                        && is_flag(h, *loc) =>
                {
                    Some(*pc)
                }
                _ => None,
            })
        });
        if let Some(lpc) = leaves {
            h.fact(
                format!("{st}.ground"),
                json!("leave"),
                "the death writes the player's standing flag 0",
                &[lpc],
            );
        }
    }
    // States in which nothing moves: no routine of a moving element runs.
    let Some(g) = f.game else { return };
    let el: BTreeMap<String, String> = h
        .out
        .routines
        .iter()
        .map(|r| (r.name.clone(), r.element.clone()))
        .collect();
    let moving = |e: &str| e == "player" || e == "camera" || e.starts_with("slots_");
    let calls: BTreeMap<String, Vec<String>> = h
        .out
        .routines
        .iter()
        .map(|r| (r.name.clone(), r.calls.clone()))
        .collect();
    let reach = |start: &str| -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut todo = vec![start.to_owned()];
        while let Some(x) = todo.pop() {
            if seen.insert(x.clone()) {
                todo.extend(calls.get(&x).cloned().unwrap_or_default());
            }
        }
        seen
    };
    let Some(d) = h
        .out
        .dispatch
        .iter()
        .find(|d| d.kind == "compare chain" && d.subject == hx(&g))
        .cloned()
    else {
        return;
    };
    let values = f.states.get(&g).cloned().unwrap_or_default();
    for v in values {
        let rs: Vec<String> = d
            .cases
            .iter()
            .filter(|(k, _)| k.split(',').any(|x| x == v.to_string()))
            .flat_map(|(_, r)| r.clone())
            .collect();
        let moves = rs.iter().any(|r| {
            reach(r)
                .iter()
                .any(|x| el.get(x).is_some_and(|e| moving(e)))
        });
        // A state with a case of its own that starts the level again is a
        // reset, not a freeze.
        if !moves {
            h.fact(
                format!("rules.{}.states.{}.freeze", byte_name(&g), state_name(v)),
                json!(true),
                "no routine that moves the player, the slots or the camera runs in this state",
                &[],
            );
        }
    }
}

/// A byte written only 0 and 1 and tested only against 0.
fn is_flag(h: &H, l: Loc) -> bool {
    let mut vals = BTreeSet::new();
    for (_, e) in h.events() {
        match e {
            Ev::Write { loc, val, .. } if *loc == l => match val.konst() {
                Some(k) => {
                    vals.insert(k);
                }
                None => return false,
            },
            Ev::Step { loc, .. } if *loc == l => return false,
            _ => {}
        }
    }
    vals == BTreeSet::from([0, 1])
}

/// Elements and each routine's: by what the routine reads and writes.
fn elements(h: &mut H, f: &mut Found) {
    let n = h.code.routines.len();
    let mut el: Vec<Option<String>> = vec![None; n];
    let mut why: Vec<String> = vec![String::new(); n];
    let vectors: BTreeSet<Va> = [0xFFFC, 0xFFFA, 0xFFFE]
        .iter()
        .map(|&v| u32::from(h.code.word(v)))
        .collect();
    // What each routine's own instructions touch.
    let mut reads: Vec<BTreeSet<Loc>> = vec![BTreeSet::new(); n];
    let mut writes: Vec<BTreeSet<Loc>> = vec![BTreeSet::new(); n];
    for (_, e) in h.events() {
        match e {
            Ev::Read { pc, loc, .. } => {
                if let Some(r) = h.code.routines.at(*pc) {
                    reads[r].insert(*loc);
                }
            }
            Ev::Write { pc, loc, .. } | Ev::Step { pc, loc, .. } => {
                if let Some(r) = h.code.routines.at(*pc) {
                    writes[r].insert(*loc);
                }
            }
            _ => {}
        }
    }
    // Calls and tail calls as the reader followed them.
    let mut callers: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    let mut callees: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for a in &h.r.acts {
        for (c, _) in &a.callers {
            if let (Some(i), Some(j)) = (
                h.code.routines.at(h.r.acts[*c].start),
                h.code.routines.at(a.start),
            ) && i != j
            {
                callers[j].insert(i);
                callees[i].insert(j);
            }
        }
    }
    let mut tiles: BTreeMap<usize, String> = BTreeMap::new();
    for t in tile_kinds(h, f) {
        if let Some(r) = h.code.routines.at(t.pc) {
            tiles
                .entry(r)
                .or_insert_with(|| format!("tile kind {}", t.kind));
        }
    }
    let in_group = |l: &Loc, f: &Found| {
        f.groups
            .iter()
            .find(|g| l.idx.is_some() && g.2.contains(&l.addr))
            .map(|g| g.0.clone())
    };
    let level_table = |l: &Loc, h: &H| {
        l.rom()
            && h.table_of(l.addr)
                .is_some_and(|t| matches!(t.asset_kind.as_str(), "metatiles" | "level-grid"))
    };
    let palette_table = |l: &Loc, h: &H| {
        l.rom()
            && h.table_of(l.addr)
                .is_some_and(|t| t.asset_kind == "palette")
    };
    let oam = |l: &Loc| (f.oam..f.oam + 0x100).contains(&l.addr);
    let queue: BTreeSet<Loc> = {
        // What a routine the NMI calls reads, when it writes the PPU's
        // address and data ports: a queue of cells to draw again.
        let nmi = u32::from(h.code.word(0xFFFA));
        let mut q = BTreeSet::new();
        for a in &h.r.acts {
            let ppu = a.events.iter().any(
                |e| matches!(e, Ev::Write { loc, .. } if loc.addr == 0x2006 || loc.addr == 0x2007),
            );
            if a.start != nmi && ppu && a.callers.iter().any(|c| h.r.acts[c.0].start == nmi) {
                for e in &a.events {
                    if let Ev::Read { loc, .. } = e
                        && loc.ram()
                        && !f.scratch.contains(loc)
                    {
                        q.insert(*loc);
                    }
                }
            }
        }
        q
    };
    for r in 0..n {
        let a = h.code.routines.addr[r];
        let rd = &reads[r];
        let wr = &writes[r];
        let set = |el: &mut Vec<Option<String>>, why: &mut Vec<String>, e: &str, w: &str| {
            if el[r].is_none() {
                el[r] = Some(e.to_owned());
                why[r] = w.to_owned();
            }
        };
        if vectors.contains(&a) || h.r.loops.contains(&a) {
            set(&mut el, &mut why, "system", "a vector or the frame loop");
        }
        if rd.iter().any(|l| l.addr == 0x4016 || l.addr == 0x4017) {
            set(&mut el, &mut why, "input", "reads the controller port");
        }
        if rd.iter().any(|l| f.counters.contains(l)) && wr.iter().any(oam) {
            set(
                &mut el,
                &mut why,
                "hud",
                "draws counters into the sprite table",
            );
        }
        if rd.iter().any(|l| f.counters.contains(l))
            && !wr.iter().any(|l| f.counters.contains(l))
            && wr.iter().all(|l| f.scratch.contains(l) || oam(l))
        {
            set(
                &mut el,
                &mut why,
                "hud",
                "reads counters, writes only scratch and sprites",
            );
        }
        if let Some(t) = tiles.get(&r) {
            set(
                &mut el,
                &mut why,
                t,
                "compares a cell's kind with a constant",
            );
        }
        if h.r.acts.iter().any(|a| {
                h.code.routines.at(a.start) == Some(r)
                    && a.events.iter().any(|e| matches!(e, Ev::Write { pc, .. } if h.code.insns.get(pc).is_some_and(|i| matches!(i.mode, Mode::IndY | Mode::IndX))))
            })
            && callees[r].iter().any(|&j| {
                writes[j].iter().any(|l| queue.contains(l))
            })
        {
            set(&mut el, &mut why, "level", "writes through a pointer and queues cells to draw again");
        }
        if wr.iter().any(|l| (0x2006..=0x2007).contains(&l.addr))
            && rd.iter().any(|l| palette_table(l, h))
        {
            set(
                &mut el,
                &mut why,
                "draw",
                "writes the palette table to the PPU",
            );
        }
        // Two ROM tables read at one index whose bytes, low and high, are
        // addresses inside the level's data: a table of the level's rows.
        let rom_idx: Vec<Va> = rd
            .iter()
            .filter(|l| l.rom() && l.idx.is_some())
            .map(|l| l.addr)
            .collect();
        let row_pointers = rom_idx.iter().any(|&lo| {
            rom_idx.iter().any(|&hi| {
                hi != lo && {
                    let w = u16::from_le_bytes([h.code.read(lo), h.code.read(hi)]);
                    h.table_of(u32::from(w))
                        .is_some_and(|t| t.asset_kind == "level-grid")
                }
            })
        });
        if rd.iter().any(|l| level_table(l, h))
            || row_pointers
            || wr.iter().any(|l| queue.contains(l))
            || rd
                .iter()
                .chain(wr.iter())
                .any(|l| l.idx.is_some() && f.cells.contains(&l.addr))
        {
            set(
                &mut el,
                &mut why,
                "level",
                "reads the level's tables or cells, or queues its redraw",
            );
        }
        if wr.iter().any(|l| (0x2000..0x2008).contains(&l.addr))
            && rd.iter().any(|l| l.rom() && h.table_of(l.addr).is_none())
        {
            set(&mut el, &mut why, "level", "writes the PPU from ROM data");
        }
        if wr.iter().any(oam) {
            set(&mut el, &mut why, "draw", "writes the sprite table");
        }
        if callees[r].iter().any(|&j| writes[j].iter().any(oam))
            && !rd.iter().any(|l| f.counters.contains(l))
        {
            set(
                &mut el,
                &mut why,
                "draw",
                "calls a routine that writes the sprite table",
            );
        }
        // G1: an entry of a jump table on a slot array is one kind of slot.
        for d in &h.out.dispatch {
            if d.kind != "jump table" {
                continue;
            }
            let rn = &h.code.routines.names[r];
            if let Some((k, _)) = d.cases.iter().find(|(_, rs)| rs.contains(rn))
                && let Some(g) = f
                    .groups
                    .iter()
                    .find(|g| g.2.iter().any(|a| d.subject == format!("${a:02X},x")))
            {
                let name = format!("{}_k{}", g.0, k.parse::<u32>().unwrap_or(0));
                set(
                    &mut el,
                    &mut why,
                    &name,
                    "an entry of the jump table on the slots' kind",
                );
            }
        }
        // The slots: written from a ROM table (their spawn) is the level
        // start; otherwise their loop.
        let slot_writes: Vec<String> = wr.iter().filter_map(|l| in_group(l, f)).collect();
        let spawns = wr.iter().any(|l| Some(*l) == f.game) && slot_writes.len() >= 2;
        if spawns {
            set(
                &mut el,
                &mut why,
                "rules",
                "writes the game state and every slot",
            );
        }
        if let Some(g) = slot_writes
            .first()
            .cloned()
            .or_else(|| rd.iter().find_map(|l| in_group(l, f)))
        {
            set(&mut el, &mut why, &g, "reads or writes the slot arrays");
        }
        if f.camera.is_some_and(|c| wr.contains(&c)) {
            set(&mut el, &mut why, "camera", "writes the camera's x");
        }
        let pw = wr.iter().filter(|l| f.player.contains(l)).count();
        if pw > 0
            && !wr
                .iter()
                .any(|l| Some(*l) == f.game && f.initial.is_some() && false)
        {
            set(&mut el, &mut why, "player", "writes the player's bytes");
        }
        if wr.iter().any(|l| Some(*l) == f.game)
            || wr.iter().any(|l| f.counters.contains(l)) && wr.iter().any(|l| f.timers.contains(l))
        {
            set(
                &mut el,
                &mut why,
                "rules",
                "writes the game state or its counters",
            );
        }
    }
    // The game state's routines that run in one case only, not the most
    // common one, are the rules'.
    for d in h.out.dispatch.clone() {
        if d.kind != "compare chain" {
            continue;
        }
        let biggest = d.cases.values().map(Vec::len).max().unwrap_or(0);
        for rs in d.cases.values() {
            if rs.len() == biggest {
                continue;
            }
            for name in rs {
                if let Some(r) = h.code.routines.index(name)
                    && el[r].as_deref() != Some("system")
                {
                    el[r] = Some("rules".into());
                    why[r] = "runs in a state of the game other than the main one".into();
                }
            }
        }
    }
    // G1: each entry of a jump table is an element of its own, unless a
    // rule above said what it is.
    for d in h.out.dispatch.clone() {
        if d.kind != "jump table" {
            continue;
        }
        for (k, rs) in &d.cases {
            for name in rs {
                if let Some(r) = h.code.routines.index(name)
                    && el[r].is_none()
                {
                    el[r] = Some(format!("dispatch_{}_{k}", d.at));
                    why[r] = format!("entry {k} of the jump table at {} on {}", d.at, d.subject);
                }
            }
        }
    }
    // A sprite writer only the counters' drawing calls is the counters'.
    // Until nothing changes, so the answer doesn't depend on where in the
    // ROM a routine is.
    loop {
        let mut changed = false;
        for r in 0..n {
            if el[r].as_deref() == Some("draw")
                && !callers[r].is_empty()
                && callers[r].iter().all(|&c| el[c].as_deref() == Some("hud"))
            {
                el[r] = Some("hud".into());
                why[r] = "writes sprites for the counters' routines only".into();
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // The rest: their callers' element when all agree, then their callees'.
    // Each until nothing changes, so the answer doesn't depend on where in
    // the ROM a routine is.
    loop {
        let mut changed = false;
        for r in 0..n {
            if el[r].is_some() {
                continue;
            }
            let cs: BTreeSet<Option<String>> = callers[r].iter().map(|&c| el[c].clone()).collect();
            if cs.len() == 1
                && let Some(Some(e)) = cs.into_iter().next()
            {
                el[r] = Some(e);
                why[r] = "called only by routines of this element".into();
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    loop {
        let before = el.clone();
        for r in 0..n {
            if before[r].is_none() {
                let mut cnt: BTreeMap<String, usize> = BTreeMap::new();
                for &j in &callees[r] {
                    if let Some(e) = &before[j] {
                        *cnt.entry(e.clone()).or_default() += 1;
                    }
                }
                if let Some((e, _)) = cnt.into_iter().max_by_key(|(_, c)| *c) {
                    el[r] = Some(e);
                    why[r] = "calls routines of this element".into();
                }
            }
        }
        if el == before {
            break;
        }
    }
    // Rows.
    for r in 0..n {
        let a = h.code.routines.addr[r];
        let calls: Vec<String> = callees[r]
            .iter()
            .map(|&j| h.code.routines.names[j].clone())
            .collect();
        h.out.routines.push(RoutineRow {
            name: h.code.routines.names[r].clone(),
            addr: crate::codemap::va_hex(a),
            size: h.code.routines.size[r],
            element: el[r].clone().unwrap_or_else(|| "unknown".into()),
            why: why[r].clone(),
            calls,
        });
    }
    let mut by_el: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in &h.out.routines {
        by_el
            .entry(row.element.clone())
            .or_default()
            .push(row.name.clone());
    }
    for (e, rs) in by_el {
        let kind = match e.as_str() {
            "player" => "player",
            e if e.starts_with("slots_") && e.contains("_k") => "slot kind",
            e if e.starts_with("slots_") => "entity kind",
            e if e.starts_with("tile kind") => "tile kind",
            e if e.starts_with("dispatch_") => "dispatch entry",
            "camera" => "camera",
            "rules" => "game rules",
            "hud" => "counters",
            "input" | "system" | "draw" | "level" => "infrastructure",
            _ => "other",
        };
        let idx: BTreeSet<usize> = rs.iter().filter_map(|n| h.code.routines.index(n)).collect();
        let mut bytes: BTreeSet<Loc> = BTreeSet::new();
        for &r in &idx {
            bytes.extend(
                reads[r]
                    .iter()
                    .chain(writes[r].iter())
                    .copied()
                    .filter(|l| l.ram() && !f.scratch.contains(l)),
            );
        }
        let rows: Vec<MapRow> = bytes
            .iter()
            .map(|l| MapRow {
                byte: hx(l),
                role: role(l, f),
                read_by: names(h, &idx, &reads, l),
                written_by: names(h, &idx, &writes, l),
            })
            .collect();
        h.out.elements.push(Element {
            name: e.clone(),
            kind: kind.into(),
            routines: rs,
            mapping: rows,
        });
    }
    for row in h.out.routines.clone() {
        if row.element == "unknown" {
            let pc = crate::codemap::va_parse(&row.addr).unwrap_or(0);
            h.open(
                format!("element of {}", row.name),
                Some(pc),
                &[],
                "no rule matches what it reads and writes, and its callers disagree",
            );
        }
    }
    // Mapping rows for the whole game, with each byte's element.
    let mut rows: BTreeMap<Loc, BTreeSet<String>> = BTreeMap::new();
    for el in &h.out.elements {
        for r in &el.mapping {
            let l = parse_loc(&r.byte);
            rows.entry(l).or_default().insert(el.name.clone());
        }
    }
    let _ = rows;
}

fn names(h: &H, idx: &BTreeSet<usize>, by: &[BTreeSet<Loc>], l: &Loc) -> Vec<String> {
    idx.iter()
        .filter(|&&r| by[r].contains(l))
        .map(|&r| h.code.routines.names[r].clone())
        .collect()
}

fn parse_loc(s: &str) -> Loc {
    let t = s.trim_start_matches('$');
    let (a, i) = match t.split_once(',') {
        Some((a, "x")) => (a, Some(Reg::X)),
        Some((a, _)) => (a, Some(Reg::Y)),
        None => (t, None),
    };
    Loc {
        addr: crate::codemap::va_parse(a).unwrap_or(0),
        idx: i,
    }
}

fn role(l: &Loc, f: &Found) -> String {
    if Some(*l) == f.frame {
        "frame counter"
    } else if Some(*l) == f.pad || Some(*l) == f.pad_new || Some(*l) == f.pad_prev {
        "input"
    } else if f.states.contains_key(l) {
        "state"
    } else if f.timers.contains(l) {
        "timer"
    } else if f.counters.contains(l) {
        "counter"
    } else if Some(*l) == f.px || Some(*l) == f.py || f.gx == Some(l.addr) && l.idx.is_some() {
        "position"
    } else if f.vy.is_some_and(|v| v.0 == *l || v.1 == *l) {
        "speed"
    } else if Some(*l) == f.camera {
        "camera"
    } else if l.idx.is_some() && f.cells.contains(&l.addr) {
        "level cells"
    } else {
        "state"
    }
    .into()
}

/// Every immediate a routine compares with, masks with or stores, and each
/// that no fact cites, as an open fact.
fn constants(h: &mut H, f: &Found) {
    let mut rows = Vec::new();
    let insns = h.all_insns();
    for (k, ins) in insns.iter().enumerate() {
        if ins.mode != Mode::Imm {
            continue;
        }
        let next = insns.get(k + 1);
        let what = match ins.mn {
            Mn::Cmp | Mn::Cpx | Mn::Cpy => "threshold",
            Mn::And => "mask",
            Mn::Adc | Mn::Sbc => "offset",
            Mn::Lda | Mn::Ldx | Mn::Ldy => match next {
                Some(n) if matches!(n.mn, Mn::Sta | Mn::Stx | Mn::Sty) => "stored",
                _ => "loaded",
            },
            _ => "operand",
        };
        let target = match (ins.mn, next) {
            (Mn::Lda | Mn::Ldx | Mn::Ldy, Some(n))
                if matches!(n.mn, Mn::Sta | Mn::Stx | Mn::Sty) =>
            {
                Some(n.text())
            }
            _ => None,
        };
        rows.push(Constant {
            routine: h.rname(ins.pc),
            pc: crate::codemap::va_hex(ins.pc),
            insn: ins.text(),
            value: ins.arg as u8,
            what: what.into(),
            then: target.unwrap_or_default(),
            cited: h.cited.contains(&ins.pc) || next.is_some_and(|n| h.cited.contains(&n.pc)),
        });
    }
    // Each constant of a game element's routine that is a threshold or a
    // value stored to state, and that no fact explains, is open: stage 1
    // aims a clip at it.
    let element_of: BTreeMap<String, String> = h
        .out
        .routines
        .iter()
        .map(|r| (r.name.clone(), r.element.clone()))
        .collect();
    for c in &rows {
        let el = element_of.get(&c.routine).map(String::as_str).unwrap_or("");
        if c.cited || matches!(el, "system" | "draw" | "input") {
            continue;
        }
        let pc = crate::codemap::va_parse(&c.pc).unwrap_or(0);
        let ins = h.code.insns.get(&pc).copied();
        let next = ins.and_then(|i| h.code.insns.get(&i.next()).copied());
        let (open, bytes): (bool, Vec<Loc>) = match c.what.as_str() {
            "threshold" => {
                // The bytes compared: the place the value was loaded from,
                // or every place it was computed from that the routine
                // itself reads. A value its caller left in the register
                // names no byte here.
                let bs: Vec<Loc> =
                    h.events()
                        .find_map(|(i, e)| match e {
                            Ev::Branch {
                                test: super::absint::Flags::Cmp { lhs, .. },
                                pc: bp,
                                ..
                            } if next.is_some_and(|n| n.pc == *bp) => {
                                let cand: Vec<Loc> = match lhs.from {
                                    Some(l) => vec![l],
                                    None => lhs.places().into_iter().collect(),
                                };
                                Some(
                                    cand.into_iter()
                                        .filter(|l| {
                                            h.r.acts[i].events.iter().any(
                                                |x| matches!(x, Ev::Read { loc, .. } if loc == l),
                                            )
                                        })
                                        .collect(),
                                )
                            }
                            _ => None,
                        })
                        .unwrap_or_default();
                let loop_count = next.is_some_and(|n| n.mode == Mode::Rel && n.arg < n.pc);
                if bs.is_empty() {
                    let why = if loop_count {
                        "a loop's count: no rule says what it counts"
                    } else {
                        "compares a value its caller left in the register; no rule says what this constant is"
                    };
                    h.open(
                        format!("{} {} {} ({el})", c.what, c.routine, c.insn),
                        Some(pc),
                        &[],
                        why,
                    );
                }
                (bs.iter().any(|l| l.ram() && !f.scratch.contains(l)), bs)
            }
            "stored" => {
                let l = next.and_then(|n| {
                    matches!(n.mode, Mode::Zp | Mode::Abs | Mode::ZpX | Mode::AbsX)
                        .then(|| Loc::at(n.arg))
                });
                (
                    l.is_some_and(|l| l.ram() && !f.scratch.contains(&l) && Some(l) != f.frame),
                    l.into_iter().collect(),
                )
            }
            _ => (false, Vec::new()),
        };
        if open {
            h.open(
                format!("{} {} {} ({el})", c.what, c.routine, c.insn),
                Some(pc),
                &bytes,
                "no rule says what this constant is in the model",
            );
        }
    }
    h.out.open.sort_by(|a, b| a.what.cmp(&b.what));
    h.out.constants = rows;
}
