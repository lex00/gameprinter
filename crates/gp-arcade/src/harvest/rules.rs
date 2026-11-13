//! The rules: the reader's events turned into routines and their roles,
//! dispatch, record tables, state machines, counters, timers, input, the
//! random routine and its callers, constants and model facts. The port of
//! gp-nes's harvest rules for what a Z80 arcade game shares with an NES
//! one (a frame counter, a state byte, slot arrays, jump tables, counters
//! and timers); the platformer rules (probes, moves, camera, level) stay
//! the NES's.

use std::collections::{BTreeMap, BTreeSet};

use super::absint::{Ev, Flags, Guard, K, Loc, Reader};
use super::board::Board;
use super::cond::{self, Cond, Rel, Subj};
use super::sem::Sem;
use super::{
    CodeMap, Constant, Dispatch, Element, Evidence, Fact, Harvest, Machine, MapRow, Open,
    RoutineRow, State, Transition, Value,
};

pub struct H<'a> {
    pub board: &'a Board,
    pub map: &'a CodeMap,
    pub r: &'a Reader<'a>,
    pub handlers: Vec<(u16, u8)>,
    pub out: Harvest,
    /// Activations that run in an interrupt: a handler and what it calls.
    in_irq: Vec<bool>,
    /// The record tables, found before the other rules: a byte of a
    /// record the code names by its address (a walk's first record, read
    /// before the loop's paths met) is that record table's field.
    tables: Vec<Table>,
    /// Each activation's events, with places put in those terms.
    acts: Vec<BTreeSet<Ev>>,
}

/// A record table: its first record, spacing, count when a loop says,
/// and the fields the code uses (offsets).
#[derive(Debug, Clone)]
struct Table {
    start: u16,
    stride: u16,
    count: Option<u8>,
    fields: BTreeSet<u16>,
    walks: Vec<u16>,
}

impl Table {
    fn name(&self) -> String {
        format!("slots_{:04X}", self.start)
    }
    fn holds(&self, l: &Loc) -> bool {
        matches!(*l, Loc::Field { addr, stride } if stride == self.stride
            && addr >= self.start && addr < self.start + self.stride)
    }
}

#[derive(Debug, Default)]
struct Found {
    frame: Option<Loc>,
    waits: BTreeMap<u16, (Loc, u16)>,
    inputs: BTreeMap<Loc, BTreeSet<u8>>,
    random: Option<Loc>,
    random_routine: Option<u16>,
    random_wrappers: BTreeSet<u16>,
    tables: Vec<Table>,
    states: BTreeMap<Loc, BTreeSet<u8>>,
    counters: BTreeSet<Loc>,
    timers: BTreeSet<Loc>,
    decimal: BTreeSet<Loc>,
    positions: BTreeSet<Loc>,
    pointers: BTreeSet<Loc>,
    flags: BTreeSet<Loc>,
    /// Routine roles, by routine start.
    roles: BTreeMap<u16, Vec<String>>,
}

impl<'a> H<'a> {
    pub fn new(
        board: &'a Board,
        map: &'a CodeMap,
        r: &'a Reader<'a>,
        handlers: &[(u16, u8)],
    ) -> Self {
        let n = r.acts.len();
        let mut in_irq: Vec<bool> = r.acts.iter().map(|a| a.interrupt).collect();
        // What an interrupt handler calls runs in the interrupt.
        loop {
            let mut changed = false;
            for i in 0..n {
                if !in_irq[i] && r.acts[i].callers.iter().any(|c| c.0 < n && in_irq[c.0]) {
                    in_irq[i] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let tables = find_tables(board, r);
        let acts = r
            .acts
            .iter()
            .map(|a| {
                a.events
                    .iter()
                    .map(|e| e.map_locs(&|l| canon(&tables, l)))
                    .collect()
            })
            .collect();
        H {
            board,
            map,
            r,
            handlers: handlers.to_vec(),
            out: Harvest::default(),
            in_irq,
            tables,
            acts,
        }
    }

    fn events(&self) -> impl Iterator<Item = (usize, &Ev)> + '_ {
        self.acts
            .iter()
            .enumerate()
            .flat_map(|(i, a)| a.iter().map(move |e| (i, e)))
    }

    fn act_events(&self, i: usize) -> &BTreeSet<Ev> {
        &self.acts[i]
    }

    /// A routine's name by its start.
    fn name(&self, start: u16) -> String {
        if start == 0 {
            "reset".into()
        } else if self.handlers.iter().any(|h| h.0 == start) {
            format!("irq_{start:04X}")
        } else {
            format!("sub_{start:04X}")
        }
    }

    /// The name of the routine an address is in.
    fn rname(&self, pc: u16) -> String {
        self.map
            .routine_of(pc)
            .map(|s| self.name(s))
            .unwrap_or_default()
    }

    /// A jump's target by name: a routine, or a place in one.
    fn target_name(&self, t: u16) -> String {
        if self.map.routines.contains_key(&t) {
            self.name(t)
        } else {
            format!("loc_{t:04X}")
        }
    }

    fn ev_at(&self, pc: u16) -> Evidence {
        Evidence {
            routine: self.rname(pc),
            pc: format!("{pc:04X}"),
            insn: self.r.insn(pc).text,
        }
    }

    fn fact(&mut self, path: impl Into<String>, value: Value, rule: &str, pcs: &[u16]) {
        let mut pcs = pcs.to_vec();
        pcs.sort_unstable();
        pcs.dedup();
        pcs.truncate(8);
        let evidence = pcs.iter().map(|&p| self.ev_at(p)).collect();
        self.out.facts.push(Fact {
            path: path.into(),
            value,
            rule: rule.into(),
            evidence,
        });
    }

    /// Every set of guards a path to an event in `act` had, its callers'
    /// with its own, up to a few levels.
    fn chains(&self, act: usize, own: &BTreeSet<Guard>) -> Vec<BTreeSet<Guard>> {
        let mut out = Vec::new();
        self.chain_into(act, own.clone(), 0, &mut out);
        out
    }

    fn chain_into(
        &self,
        act: usize,
        g: BTreeSet<Guard>,
        depth: usize,
        out: &mut Vec<BTreeSet<Guard>>,
    ) {
        let callers = &self.r.acts[act].callers;
        if callers.is_empty() || depth >= 4 || out.len() >= 16 {
            out.push(g);
            return;
        }
        for (c, _, cg) in callers {
            if *c >= self.r.acts.len() {
                continue;
            }
            let mut n = g.clone();
            n.extend(cg.iter().map(|x| x.map_locs(&|l| canon(&self.tables, l))));
            self.chain_into(*c, n, depth + 1, out);
        }
    }
}

fn int(v: impl Into<i64>) -> Value {
    Value::Int(v.into())
}

fn s(v: impl Into<String>) -> Value {
    Value::Str(v.into())
}

pub fn run(h: &mut H) {
    let mut f = Found::default();
    interrupts(h, &mut f);
    frame(h, &mut f);
    input(h, &mut f);
    random(h, &mut f);
    tables(h, &mut f);
    states(h, &mut f);
    dispatch(h, &mut f);
    machines(h, &mut f);
    counters(h, &mut f);
    elements(h, &mut f);
    routines(h, &f);
    open(h);
    constants(h);
}

/// IM 2 through the CTC: the vector table and each channel's handler.
fn interrupts(h: &mut H, _f: &mut Found) {
    let r = h.r;
    if let Some(&m) = r.im.iter().max() {
        h.fact(
            "hardware.interrupt_mode",
            int(m),
            "the IM instruction the reset code runs",
            &[],
        );
    }
    if r.im.contains(&2) {
        for &i in &r.i_reg {
            for &v in &r.ctc_vectors {
                let at = (u16::from(i) << 8) | u16::from(v);
                h.fact(
                    "hardware.vector_table",
                    s(format!("{at:04X}")),
                    "I's value and the vector written to the CTC's channel 0",
                    &[],
                );
            }
        }
    }
    for (addr, ch) in h.handlers.clone() {
        let name = h.name(addr);
        h.fact(
            format!("hardware.interrupts.ctc{ch}"),
            s(name),
            "the word for this CTC channel in the vector table",
            &[addr],
        );
    }
}

/// The frame counter (a byte a handler adds 1 to on every interrupt) and
/// the waits for it (a loop reading a byte only a handler writes).
fn frame(h: &mut H, f: &mut Found) {
    let mut found = None;
    for (i, e) in h.events() {
        if let Ev::Step {
            loc,
            by: 1,
            guards,
            pc,
        } = e
            && h.r.acts[i].interrupt
            && guards.is_empty()
            && matches!(loc, Loc::At(a) if h.board.is_ram(*a))
            && found.is_none()
        {
            found = Some((*loc, *pc));
        }
    }
    if let Some((l, pc)) = found {
        f.frame = Some(l);
        h.fact(
            "hardware.frame_counter",
            s(l.hex()),
            "a byte an interrupt handler adds 1 to on every interrupt",
            &[pc],
        );
    }
    // Places the handlers (and what they call) write.
    let mut irq_writes: BTreeMap<Loc, u16> = BTreeMap::new();
    for (i, e) in h.events() {
        if let Ev::Write { loc, pc, .. } | Ev::Step { loc, pc, .. } = e
            && h.in_irq[i]
        {
            irq_writes.entry(*loc).or_insert(*pc);
        }
    }
    // A wait: a branch back to a read of such a place, nothing written
    // between.
    for (i, e) in h.events() {
        let Ev::Branch { pc: bp, .. } = e else {
            continue;
        };
        let Sem::Jp(Some(_), t) = h.r.insn(*bp).sem else {
            continue;
        };
        // A busy wait is a tight loop.
        if t > *bp || *bp - t > 16 {
            continue;
        }
        for e2 in h.act_events(i) {
            if let Ev::Read { pc: rp, loc, .. } = e2
                && *rp >= t
                && *rp < *bp
                && matches!(loc, Loc::At(_) | Loc::Word(_) | Loc::Field { .. })
                && let Some(&wpc) = irq_writes.get(loc)
                && h.map.routine_of(wpc) != h.map.routine_of(*bp)
                && !h.act_events(i).iter().any(|x| {
                    matches!(x, Ev::Write { pc, .. } | Ev::Step { pc, .. } if *pc >= t && *pc < *bp)
                })
            {
                f.waits.insert(*bp, (*loc, wpc));
            }
        }
    }
    let waits: Vec<(u16, (Loc, u16))> = f.waits.iter().map(|(a, b)| (*a, *b)).collect();
    for (bp, (loc, wpc)) in waits {
        let Some(rt) = h.map.routine_of(bp) else {
            continue;
        };
        let writer = h.rname(wpc);
        let in_irq_wait = h.handlers.iter().any(|x| x.0 == rt);
        f.roles
            .entry(rt)
            .or_default()
            .push(format!("waits for {} (written by {writer})", loc.hex()));
        if Some(loc) == f.frame && !in_irq_wait {
            let name = h.name(rt);
            h.fact(
                "hardware.frame_wait",
                s(name),
                "a loop that reads the frame counter until the interrupt changes it",
                &[bp],
            );
        }
    }
}

/// Input: bytes made from the board's input ports.
fn input(h: &mut H, f: &mut Found) {
    let mut readers: BTreeMap<u8, BTreeSet<u16>> = BTreeMap::new();
    for (_, e) in h.events() {
        match e {
            Ev::In { pc, port: Some(p) } if h.board.is_input(*p) => {
                readers.entry(*p).or_default().insert(*pc);
            }
            Ev::Write { loc, val, .. } if matches!(loc, Loc::At(a) if h.board.is_ram(*a)) => {
                for p in val.places() {
                    if let Loc::Port(n) = p
                        && h.board.is_input(n)
                    {
                        f.inputs.entry(*loc).or_default().insert(n);
                    }
                }
            }
            _ => {}
        }
    }
    for (p, pcs) in &readers {
        let pcs: Vec<u16> = pcs.iter().copied().collect();
        let names: BTreeSet<String> = pcs.iter().map(|&pc| h.rname(pc)).collect();
        h.fact(
            format!("input.ports.p{p:02X}"),
            Value::List(names.into_iter().map(s).collect()),
            "the routines that read this input port",
            &pcs,
        );
        for &pc in &pcs {
            if let Some(rt) = h.map.routine_of(pc) {
                let role = format!("reads input port {p:02X}");
                let v = f.roles.entry(rt).or_default();
                if !v.contains(&role) {
                    v.push(role);
                }
            }
        }
    }
    let ins: Vec<(Loc, BTreeSet<u8>)> = f.inputs.iter().map(|(a, b)| (*a, b.clone())).collect();
    for (l, ports) in ins {
        let pcs: Vec<u16> = h
            .events()
            .filter_map(|(_, e)| match e {
                Ev::Write { loc, pc, .. } if *loc == l => Some(*pc),
                _ => None,
            })
            .collect();
        let ps: Vec<String> = ports.iter().map(|p| format!("{p:02X}")).collect();
        h.fact(
            format!("input.bytes.{}", l.hex()),
            s(format!("port {}", ps.join(", "))),
            "a byte written from an input port's value",
            &pcs,
        );
    }
}

/// The random register: a byte written from the R register and its own
/// old value; its writer is the random routine; a routine that calls it
/// and keeps nothing is a wrapper; every call site is listed.
fn random(h: &mut H, f: &mut Found) {
    let mut found: Option<(Loc, u16)> = None;
    let mut seeds = Vec::new();
    for (_, e) in h.events() {
        if let Ev::Write { loc, val, pc, .. } = e
            && matches!(loc, Loc::At(a) if h.board.is_ram(*a))
        {
            let p = val.places();
            if p.contains(&Loc::Refresh) && p.contains(loc) {
                if found.is_none() {
                    found = Some((*loc, *pc));
                }
            } else if p.contains(&Loc::Refresh) {
                seeds.push((*loc, *pc));
            }
        }
    }
    let Some((l, pc)) = found else { return };
    let Some(rt) = h.map.routine_of(pc) else {
        return;
    };
    f.random = Some(l);
    f.random_routine = Some(rt);
    let rname = h.name(rt);
    h.fact(
        "random.byte",
        s(l.hex()),
        "a byte written from the R register and its own last value",
        &[pc],
    );
    h.fact(
        "random.routine",
        s(rname.clone()),
        "the routine that writes the random byte",
        &[pc],
    );
    seeds.sort_unstable();
    seeds.dedup();
    for (sl, spc) in seeds {
        if sl == l {
            h.fact(
                "random.seed",
                s(h.rname(spc)),
                "the random byte set from the R register alone",
                &[spc],
            );
        }
    }
    f.roles
        .entry(rt)
        .or_default()
        .push(format!("random: steps {} with the R register", l.hex()));
    // Call sites, by callee.
    let mut sites: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
    for (_, e) in h.events() {
        if let Ev::Call { pc, to, .. } = e {
            sites.entry(*to).or_default().insert(*pc);
        }
    }
    // Wrappers: a routine that calls the random routine and writes no RAM
    // of its own.
    let callers: BTreeSet<u16> = sites
        .get(&rt)
        .into_iter()
        .flatten()
        .filter_map(|&pc| h.map.routine_of(pc))
        .collect();
    for c in callers {
        let writes = h.events().any(|(_, e)| {
            matches!(e, Ev::Write { pc, loc, .. } | Ev::Step { pc, loc, .. }
                if h.map.routine_of(*pc) == Some(c) && matches!(loc, Loc::At(a) if h.board.is_ram(*a)))
        });
        // A wrapper compares the result with an argument.
        let compares = h.events().any(|(_, e)| {
            matches!(e, Ev::Branch { pc, test: Flags::Cmp { .. }, .. } if h.map.routine_of(*pc) == Some(c))
        });
        if !writes && compares && c != rt {
            f.random_wrappers.insert(c);
            let n = h.name(c);
            h.fact(
                format!("random.wrappers.{n}"),
                s(format!("calls {rname} and bounds the result")),
                "a routine that calls the random routine, compares, and writes nothing",
                &[c],
            );
            f.roles
                .entry(c)
                .or_default()
                .push(format!("random below a bound (calls {rname})"));
        }
    }
    let mut all: Vec<(u16, u16)> = Vec::new();
    for t in std::iter::once(rt).chain(f.random_wrappers.iter().copied()) {
        for &pc in sites.get(&t).into_iter().flatten() {
            if h.map.routine_of(pc) != Some(rt)
                && !f
                    .random_wrappers
                    .contains(&h.map.routine_of(pc).unwrap_or(0))
            {
                all.push((pc, t));
            }
        }
    }
    all.sort_unstable();
    let list: Vec<Value> = all
        .iter()
        .map(|(pc, t)| s(format!("{} at {pc:04X} ({})", h.rname(*pc), h.name(*t))))
        .collect();
    let pcs: Vec<u16> = all.iter().map(|x| x.0).collect();
    h.fact(
        "random.callers",
        Value::List(list),
        "every call of the random routine or a wrapper, outside them",
        &pcs,
    );
}

/// Record tables: places a register walking records reads and writes,
/// grouped by spacing; a loop's walk gives the first record and the count.
/// A loop's walk: its head, first record, stride and djnz count.
type Walk = (u16, u16, u16, Option<u8>);

fn find_tables(board: &Board, r: &Reader) -> Vec<Table> {
    let mut fields: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
    let mut walks: Vec<Walk> = Vec::new();
    for e in r.acts.iter().flat_map(|a| a.events.iter()) {
        match e {
            Ev::Read { loc, .. } | Ev::Write { loc, .. } | Ev::Step { loc, .. } => {
                if let Loc::Field { addr, stride } = *loc
                    && board.is_ram(addr)
                {
                    fields.entry(stride).or_default().insert(addr);
                }
            }
            Ev::Walk {
                head,
                base,
                stride,
                count,
                ..
            } if board.is_ram(*base) && (2..=0x80).contains(stride) => {
                walks.push((*head, *base, *stride, *count));
            }
            _ => {}
        }
    }
    // A table is where a loop walks: walks of one stride whose first
    // records are within a record of each other are one table. Its first record is where the longest walk (the
    // largest djnz count) starts, else the lowest.
    walks.sort_by_key(|w| (w.2, w.1));
    let mut groups: Vec<Vec<Walk>> = Vec::new();
    for &w in &walks {
        let (_, base, stride, _) = w;
        match groups
            .iter_mut()
            .find(|g| g[0].2 == stride && g.iter().any(|x| base.abs_diff(x.1) <= stride))
        {
            Some(g) => g.push(w),
            None => groups.push(vec![w]),
        }
    }
    let mut out: Vec<Table> = Vec::new();
    for g in groups {
        let stride = g[0].2;
        let best = g
            .iter()
            .max_by_key(|w| (w.3.unwrap_or(0), std::cmp::Reverse(w.1)))
            .expect("a group has a walk");
        let count = g.iter().filter_map(|w| w.3).filter(|&c| c > 1).max();
        out.push(Table {
            start: best.1,
            stride,
            count: if best.3.is_some_and(|c| c > 1) {
                best.3
            } else {
                count
            },
            fields: BTreeSet::new(),
            walks: g.iter().map(|w| w.0).collect(),
        });
    }
    // Its fields: the places of that stride near it, by offset in a record
    // (`(ix-4)` is the record before's byte 4).
    for t in &mut out {
        t.walks.sort_unstable();
        t.walks.dedup();
        let n = i32::from(t.count.unwrap_or(1).max(1));
        for &a in fields.get(&t.stride).into_iter().flatten() {
            let d = i32::from(a) - i32::from(t.start);
            let s = i32::from(t.stride);
            if d >= -s && d < n * s {
                t.fields.insert(d.rem_euclid(s) as u16);
            }
        }
    }
    out.retain(|t| !t.fields.is_empty());
    out.sort_by_key(|t| (t.start, t.stride));
    out
}

/// A byte of a record table's records, or a field of one put by its
/// address, as the table's field at its first record.
fn canon(tables: &[Table], l: Loc) -> Loc {
    let (a, want) = match l {
        Loc::At(a) => (a, None),
        Loc::Field { addr, stride } => (addr, Some(stride)),
        _ => return l,
    };
    for t in tables {
        // A field of a finer stride (a record picked by an index the reader
        // doesn't know) is a field of the table when it lands on one.
        if want.is_some_and(|s| s == 0 || s > t.stride || t.stride % s != 0) {
            continue;
        }
        let n = i32::from(t.count.unwrap_or(1).max(1));
        let s = i32::from(t.stride);
        let d = i32::from(a) - i32::from(t.start);
        let lo = if want.is_some() { -s } else { 0 };
        if d < lo || d >= n * s {
            continue;
        }
        let off = d.rem_euclid(s) as u16;
        if t.fields.contains(&off) {
            return Loc::Field {
                addr: t.start + off,
                stride: t.stride,
            };
        }
    }
    l
}

/// The record tables as facts.
fn tables(h: &mut H, f: &mut Found) {
    let out = h.tables.clone();
    for t in &out {
        let name = t.name();
        let path = format!("entities.{name}");
        h.fact(
            format!("{path}.base"),
            s(format!("{:04X}", t.start)),
            "the first record a loop walks with a register stepped by the stride",
            &t.walks,
        );
        h.fact(
            format!("{path}.stride"),
            int(t.stride),
            "the step between records",
            &t.walks,
        );
        if let Some(c) = t.count {
            h.fact(
                format!("{path}.count"),
                int(c),
                "the djnz count the walk starts with",
                &t.walks,
            );
            h.fact(
                format!("{path}.end"),
                s(format!(
                    "{:04X}",
                    u32::from(t.start) + u32::from(t.stride) * u32::from(c) - 1
                )),
                "the last byte of the last record",
                &t.walks,
            );
        }
        h.fact(
            format!("{path}.fields"),
            Value::List(t.fields.iter().map(|&o| int(o)).collect()),
            "the offsets the code reads and writes in a record",
            &[],
        );
    }
    f.tables = out;
}

/// State bytes: tested by value (a compare, or a jump table's index) and
/// written with constants. Counters, timers, decimal digits and positions
/// by how they change.
fn states(h: &mut H, f: &mut Found) {
    let mut tested: BTreeMap<Loc, BTreeSet<u8>> = BTreeMap::new();
    let mut written: BTreeMap<Loc, BTreeSet<u8>> = BTreeMap::new();
    let mut other: BTreeSet<Loc> = BTreeSet::new();
    let mut stepped: BTreeSet<Loc> = BTreeSet::new();
    let mut ups: BTreeSet<Loc> = BTreeSet::new();
    let mut downs: BTreeSet<Loc> = BTreeSet::new();
    let mut step_tested: BTreeSet<Loc> = BTreeSet::new();
    let mut cmp_any: BTreeSet<Loc> = BTreeSet::new();
    let mut bits: BTreeSet<Loc> = BTreeSet::new();
    let mut self_sum: BTreeSet<Loc> = BTreeSet::new();
    let mut indexes: BTreeSet<Loc> = BTreeSet::new();
    let mut big_steps: BTreeSet<Loc> = BTreeSet::new();
    let mut decimal_pcs: BTreeSet<(usize, u16)> = BTreeSet::new();
    for (i, e) in h.events() {
        if let Ev::Decimal { pc } = e {
            decimal_pcs.insert((i, *pc));
        }
    }
    for (i, e) in h.events() {
        match e {
            Ev::Branch { pc, cc, test, .. } => {
                let g = Guard {
                    pc: *pc,
                    cc: *cc,
                    taken: true,
                    test: test.clone(),
                };
                if let Some(c) = cond::of(&g) {
                    if let Subj::Byte(l, off) = c.subj {
                        if c.stepped {
                            step_tested.insert(l);
                        } else if off == 0 && matches!(c.rel, Rel::Eq | Rel::Ne) {
                            tested.entry(l).or_default().insert(c.k);
                        }
                        if c.k != 0 {
                            cmp_any.insert(l);
                        }
                    }
                    if let Subj::Bit(l, _) | Subj::Mask(l, _) = c.subj {
                        bits.insert(l);
                    }
                }
            }
            Ev::Indirect { to, .. } => {
                for (_, case) in to {
                    if let Some((l, k)) = case {
                        tested.entry(*l).or_default().insert(*k);
                        indexes.insert(*l);
                    }
                }
            }
            Ev::Write {
                loc,
                val,
                pc,
                added,
                ..
            } => {
                if decimal_pcs
                    .iter()
                    .any(|&(a, d)| a == i && *pc > d && *pc - d <= 3)
                    && matches!(loc, Loc::At(a) | Loc::Field { addr: a, .. } if h.board.is_ram(*a))
                {
                    f.decimal.insert(*loc);
                }
                match val.k8() {
                    Some(k) => {
                        written.entry(*loc).or_default().insert(k);
                    }
                    None => {
                        if *added && val.places().contains(loc) {
                            self_sum.insert(*loc);
                        }
                        other.insert(*loc);
                    }
                }
            }
            Ev::Step { loc, by, .. } => {
                stepped.insert(*loc);
                if by.unsigned_abs() > 1 {
                    big_steps.insert(*loc);
                }
                if *by > 0 {
                    ups.insert(*loc);
                } else {
                    downs.insert(*loc);
                }
            }
            _ => {}
        }
    }
    let ram = |l: &Loc, h: &H| match l {
        Loc::At(a) | Loc::Field { addr: a, .. } => h.board.is_ram(*a),
        _ => false,
    };
    for (l, w) in &written {
        // A jump table's index may also be copied from another byte (a
        // saved state put back).
        let copied_ok = indexes.contains(l) && w.len() >= 2;
        if !ram(l, h)
            || stepped.contains(l)
            || (other.contains(l) && !copied_ok)
            || Some(*l) == f.frame
        {
            continue;
        }
        let t = tested.get(l).cloned().unwrap_or_default();
        let all: BTreeSet<u8> = t.union(w).copied().collect();
        if t.is_empty() || all.len() < 2 || !(t.iter().any(|&k| k != 0) || all.len() >= 3) {
            if w.iter().all(|&k| k <= 1) && w.len() == 2 {
                f.flags.insert(*l);
            }
            continue;
        }
        f.states.insert(*l, all);
    }
    for l in &stepped {
        if !ram(l, h) || Some(*l) == f.frame || Some(*l) == f.random {
            continue;
        }
        if step_tested.contains(l) && !ups.contains(l) {
            f.timers.insert(*l);
            continue;
        }
        if big_steps.contains(l) || self_sum.contains(l) {
            f.positions.insert(*l);
            continue;
        }
        if other.contains(l) && !f.decimal.contains(l) {
            continue;
        }
        // Stepped by 1 (either way) and set by constants: a counter (lives,
        // levels, a count of objects).
        f.counters.insert(*l);
    }
    // A place written with its own value plus something not constant (`add
    // a,(ix+3); ld (ix+3),a`) moves: a position.
    for l in &self_sum {
        if ram(l, h)
            && !f.decimal.contains(l)
            && Some(*l) != f.random
            && !f.timers.contains(l)
            && !bits.contains(l)
            && !f.states.contains_key(l)
        {
            f.positions.insert(*l);
        }
    }
    let _ = cmp_any;
    for (_, e) in h.events() {
        if let Ev::Write {
            loc: Loc::Word(a),
            val,
            ..
        } = e
            && h.board.is_ram(*a)
            && val.konst().is_some()
        {
            f.pointers.insert(Loc::Word(*a));
        }
    }
    f.counters.retain(|l| !f.decimal.contains(l));
}

fn state_name(v: u8) -> String {
    format!("s{v}")
}

/// A place's name in the model: a record table's field by its table.
fn place_path(l: &Loc, f: &Found) -> (String, String) {
    if let Loc::Field { addr, .. } = l
        && let Some(t) = f.tables.iter().find(|t| t.holds(l))
    {
        let off = addr - t.start;
        return (
            format!("entities.{}.machine_{off:02X}", t.name()),
            format!("state_{}_{off:02X}", t.name()),
        );
    }
    let a = l.addr().unwrap_or(0);
    (format!("rules.state_{a:04X}"), format!("state_{a:04X}"))
}

/// Jump tables and compare chains: which code runs for each value of a
/// byte.
fn dispatch(h: &mut H, f: &mut Found) {
    let mut seen: BTreeSet<u16> = BTreeSet::new();
    let mut rows = Vec::new();
    for (_, e) in h.events() {
        if let Ev::Indirect { pc, target, to } = e
            && to.len() >= 2
            && seen.insert(*pc)
        {
            let subject = match &target.k {
                K::TabW {
                    at,
                    by: Some((l, sh)),
                } => {
                    format!("{} (table {at:04X}, index shifted {sh})", l.hex())
                }
                K::TabW { at, by: None } => format!("? (table {at:04X})"),
                _ => "a pointer".into(),
            };
            let mut cases: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for (i, (t, case)) in to.iter().enumerate() {
                let key = case.map_or(i.to_string(), |c| c.1.to_string());
                cases.entry(key).or_default().push(h.target_name(*t));
            }
            rows.push((*pc, subject, cases, "jump table"));
        }
    }
    // Compare chains and bit chains, per routine and place.
    let mut chains: BTreeMap<(u16, Loc, bool), BTreeMap<String, BTreeSet<u16>>> = BTreeMap::new();
    for (_, e) in h.events() {
        let Ev::Branch { pc, cc, test, .. } = e else {
            continue;
        };
        let Some(rt) = h.map.routine_of(*pc) else {
            continue;
        };
        let ins = h.r.insn(*pc);
        let Sem::Jp(Some(_), t) = ins.sem else {
            continue;
        };
        let next = pc.wrapping_add(u16::from(ins.len));
        let g = Guard {
            pc: *pc,
            cc: *cc,
            taken: true,
            test: test.clone(),
        };
        let Some(c) = cond::of(&g) else { continue };
        match (&c.subj, c.rel) {
            (Subj::Byte(l, 0), Rel::Eq | Rel::Ne) if !c.stepped => {
                let dest = if c.rel == Rel::Eq { t } else { next };
                chains
                    .entry((rt, *l, false))
                    .or_default()
                    .entry(c.k.to_string())
                    .or_default()
                    .insert(dest);
            }
            (Subj::Bit(l, n), _) => {
                let dest = if c.rel == Rel::Ne { t } else { next };
                chains
                    .entry((rt, *l, true))
                    .or_default()
                    .entry(format!("bit {n}"))
                    .or_default()
                    .insert(dest);
            }
            (Subj::Mask(l, m), _) => {
                let dest = if c.rel == Rel::Ne { t } else { next };
                chains
                    .entry((rt, *l, true))
                    .or_default()
                    .entry(format!("& ${m:02X}"))
                    .or_default()
                    .insert(dest);
            }
            _ => {}
        }
    }
    for ((rt, l, bit), cases) in chains {
        // A dispatch sends values to different places.
        let dests: BTreeSet<u16> = cases.values().flatten().copied().collect();
        let enough = if bit { 2 } else { 3 };
        if cases.len() < enough || dests.len() < 2 || !matches!(l, Loc::At(_) | Loc::Field { .. }) {
            continue;
        }
        let named: BTreeMap<String, Vec<String>> = cases
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().map(|t| h.target_name(t)).collect()))
            .collect();
        rows.push((
            rt,
            l.hex(),
            named,
            if bit {
                "bit or mask chain"
            } else {
                "compare chain"
            },
        ));
    }
    rows.sort_by_key(|r| r.0);
    for (at, subject, cases, kind) in rows {
        if let Some(rt) = h.map.routine_of(at) {
            f.roles
                .entry(rt)
                .or_default()
                .push(format!("{kind} at {at:04X} on {subject}"));
        }
        h.out.dispatch.push(Dispatch {
            kind: kind.into(),
            at: format!("{at:04X}"),
            subject,
            cases,
        });
    }
}

/// State machines: each state byte's values, and its transitions (each
/// constant write, from the values the paths to it had tested).
fn machines(h: &mut H, f: &mut Found) {
    let states = f.states.clone();
    for (l, values) in &states {
        let (path, name) = place_path(l, f);
        let mut m = Machine {
            byte: l.hex(),
            name,
            initial: None,
            path: path.clone(),
            states: values
                .iter()
                .map(|&v| State {
                    value: v,
                    name: state_name(v),
                })
                .collect(),
            transitions: Vec::new(),
        };
        let mut pcs = Vec::new();
        let mut seen: BTreeSet<(String, String, u16)> = BTreeSet::new();
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
            if loc != l {
                continue;
            }
            let Some(to) = val.k8() else { continue };
            // The power-on value: written by the reset's own activation.
            if h.r.acts[i].start == 0 && guards.is_empty() && m.initial.is_none() {
                m.initial = Some(state_name(to));
                continue;
            }
            for g in h.chains(i, guards) {
                let conds: Vec<Cond> = g.iter().filter_map(cond::of).collect();
                let froms: BTreeSet<u8> = conds
                    .iter()
                    .filter_map(|c| c.equals())
                    .filter(|(p, _)| p == l)
                    .map(|(_, v)| v)
                    .collect();
                let from = if froms.len() == 1 {
                    state_name(*froms.iter().next().unwrap_or(&0))
                } else {
                    "any".into()
                };
                if !seen.insert((from.clone(), state_name(to), *pc)) {
                    continue;
                }
                let others: Vec<String> = conds
                    .iter()
                    .filter(|c| c.place() != Some(*l))
                    .map(|c| c.text())
                    .collect();
                m.transitions.push(Transition {
                    from,
                    to: state_name(to),
                    at: format!("{pc:04X}"),
                    routine: h.rname(*pc),
                    guards: others,
                    after: None,
                });
                pcs.push(*pc);
            }
        }
        m.transitions
            .sort_by(|a, b| (&a.from, &a.to, &a.at).cmp(&(&b.from, &b.to, &b.at)));
        h.fact(
            format!("{path}.states"),
            Value::List(values.iter().map(|&v| s(state_name(v))).collect()),
            "the values the code tests the byte for and writes to it",
            &pcs,
        );
        if let Some(i) = &m.initial {
            h.fact(
                format!("{path}.initial"),
                s(i.clone()),
                "the value the reset code writes",
                &[],
            );
        }
        h.out.machines.push(m);
    }
}

/// Counters, timers, decimal counters, positions and pointers as facts.
fn counters(h: &mut H, f: &mut Found) {
    let pc_of = |h: &H, l: &Loc| -> Vec<u16> {
        h.events()
            .filter_map(|(_, e)| match e {
                Ev::Step { loc, pc, .. } | Ev::Write { loc, pc, .. } if loc == l => Some(*pc),
                _ => None,
            })
            .collect()
    };
    for l in f.counters.clone() {
        let pcs = pc_of(h, &l);
        h.fact(
            format!("counters.c_{}", l.hex()),
            s(l.hex()),
            "a byte changed only by 1 at a time one way, and set by constants",
            &pcs,
        );
    }
    for l in f.timers.clone() {
        let pcs = pc_of(h, &l);
        // The constants it is loaded with.
        let loads: BTreeSet<u8> = h
            .events()
            .filter_map(|(_, e)| match e {
                Ev::Write { loc, val, .. } if *loc == l => val.k8(),
                _ => None,
            })
            .collect();
        h.fact(
            format!("timers.t_{}", l.hex()),
            Value::List(loads.iter().map(|&k| int(k)).collect()),
            "a byte counted down and tested for 0; the values it is loaded with",
            &pcs,
        );
    }
    for l in f.decimal.clone() {
        let pcs = pc_of(h, &l);
        h.fact(
            format!("counters.d_{}", l.hex()),
            s(l.hex()),
            "a decimal digit: added to and adjusted with daa",
            &pcs,
        );
    }
    for l in f.positions.clone() {
        let pcs = pc_of(h, &l);
        h.fact(
            format!("positions.p_{}", l.hex()),
            s(l.hex()),
            "a byte added to by amounts and stepped both ways",
            &pcs,
        );
    }
    for l in f.pointers.clone() {
        let pcs = pc_of(h, &l);
        h.fact(
            format!("pointers.w_{}", l.hex()),
            s(l.hex()),
            "a 16-bit variable set to addresses",
            &pcs,
        );
    }
}

fn role(l: &Loc, f: &Found) -> &'static str {
    if Some(*l) == f.frame {
        "frame counter"
    } else if Some(*l) == f.random {
        "random"
    } else if f.inputs.contains_key(l) {
        "input"
    } else if f.states.contains_key(l) {
        "state"
    } else if f.timers.contains(l) {
        "timer"
    } else if f.counters.contains(l) {
        "counter"
    } else if f.decimal.contains(l) {
        "decimal counter"
    } else if f.positions.contains(l) {
        "position"
    } else if f.pointers.contains(l) {
        "pointer"
    } else if f.flags.contains(l) {
        "flag"
    } else {
        ""
    }
}

/// Elements: each record table with the routines that touch its fields;
/// the RAM's roles as one more, `globals`.
fn elements(h: &mut H, f: &mut Found) {
    let mut readers: BTreeMap<Loc, BTreeSet<u16>> = BTreeMap::new();
    let mut writers: BTreeMap<Loc, BTreeSet<u16>> = BTreeMap::new();
    for (_, e) in h.events() {
        match e {
            Ev::Read { pc, loc, .. } => {
                if let Some(r) = h.map.routine_of(*pc) {
                    readers.entry(*loc).or_default().insert(r);
                }
            }
            Ev::Write { pc, loc, .. } | Ev::Step { pc, loc, .. } => {
                if let Some(r) = h.map.routine_of(*pc) {
                    writers.entry(*loc).or_default().insert(r);
                }
            }
            _ => {}
        }
    }
    let names = |h: &H, s: Option<&BTreeSet<u16>>| -> Vec<String> {
        s.into_iter().flatten().map(|&r| h.name(r)).collect()
    };
    for t in f.tables.clone() {
        let mut routines: BTreeSet<u16> = BTreeSet::new();
        let mut mapping = Vec::new();
        for (l, rs) in readers.iter().chain(writers.iter()) {
            if t.holds(l) {
                routines.extend(rs.iter().copied());
            }
        }
        let locs: BTreeSet<Loc> = readers
            .keys()
            .chain(writers.keys())
            .filter(|l| t.holds(l))
            .copied()
            .collect();
        for l in locs {
            let Loc::Field { addr, .. } = l else { continue };
            mapping.push(MapRow {
                byte: format!("+{:02X}", addr - t.start),
                role: role(&l, f).into(),
                read_by: names(h, readers.get(&l)),
                written_by: names(h, writers.get(&l)),
            });
        }
        for &r in &routines {
            f.roles
                .entry(r)
                .or_default()
                .push(format!("reads or writes {}", t.name()));
        }
        h.out.elements.push(Element {
            name: t.name(),
            kind: format!("record table, {} bytes a record", t.stride),
            routines: routines.iter().map(|&r| h.name(r)).collect(),
            mapping,
        });
    }
    // Globals with a role.
    let mut mapping = Vec::new();
    let mut routines: BTreeSet<u16> = BTreeSet::new();
    let locs: BTreeSet<Loc> = readers
        .keys()
        .chain(writers.keys())
        .filter(|l| matches!(l, Loc::At(a) | Loc::Word(a) if h.board.is_ram(*a)))
        .copied()
        .collect();
    for l in locs {
        let r = role(&l, f);
        if r.is_empty() {
            continue;
        }
        routines.extend(writers.get(&l).into_iter().flatten().copied());
        mapping.push(MapRow {
            byte: l.hex(),
            role: r.into(),
            read_by: names(h, readers.get(&l)),
            written_by: names(h, writers.get(&l)),
        });
    }
    if !mapping.is_empty() {
        h.out.elements.push(Element {
            name: "globals".into(),
            kind: "work RAM".into(),
            routines: routines.iter().map(|&r| h.name(r)).collect(),
            mapping,
        });
    }
}

/// The code map's routines, with what the rules found each one does.
fn routines(h: &mut H, f: &Found) {
    let mut calls: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
    for (_, e) in h.events() {
        if let Ev::Call { pc, to, .. } = e
            && let Some(r) = h.map.routine_of(*pc)
        {
            calls.entry(r).or_default().insert(*to);
        }
    }
    let element_of = |r: u16| -> String {
        h.out
            .elements
            .iter()
            .find(|e| e.name != "globals" && e.routines.contains(&h.name(r)))
            .map(|e| e.name.clone())
            .unwrap_or_default()
    };
    let mut rows = Vec::new();
    for (&start, why) in &h.map.routines {
        let mut w = vec![(*why).to_string()];
        if let Some(&(_, ch)) = h.handlers.iter().find(|x| x.0 == start) {
            w[0] = format!("interrupt handler, CTC channel {ch}");
        }
        let mut roles: Vec<String> = f.roles.get(&start).cloned().unwrap_or_default();
        roles.dedup();
        w.extend(roles);
        rows.push(RoutineRow {
            name: h.name(start),
            addr: format!("{start:04X}"),
            size: h.map.size(start),
            element: element_of(start),
            why: w.join("; "),
            calls: calls
                .get(&start)
                .into_iter()
                .flatten()
                .map(|&t| h.name(t))
                .collect(),
        });
    }
    h.out.routines = rows;
}

/// What the code leaves open: jumps through a value the reader couldn't
/// name.
fn open(h: &mut H) {
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    for (_, e) in h.events() {
        if let Ev::Indirect { pc, target, to } = e
            && to.is_empty()
            && seen.insert(*pc)
        {
            let bytes: Vec<String> = h.board.bytes(*pc)[..usize::from(h.r.insn(*pc).len)]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect();
            let why = match &target.k {
                K::Top(t) if !t.is_empty() => {
                    let p: Vec<String> = t.iter().map(Loc::hex).collect();
                    format!(
                        "the target is made from {} and the reader can't name it",
                        p.join(", ")
                    )
                }
                _ => "the target isn't known on this path".into(),
            };
            rows.push(Open {
                what: format!("jump through a register at {pc:04X}"),
                routine: h.rname(*pc),
                pc: format!("{pc:04X}"),
                bytes,
                why,
            });
        }
    }
    rows.sort_by(|a, b| a.what.cmp(&b.what));
    h.out.open = rows;
}

/// Constants: thresholds, masks and stored values, and whether a fact
/// cites each.
fn constants(h: &mut H) {
    let cited: BTreeSet<String> = h
        .out
        .facts
        .iter()
        .flat_map(|f| f.evidence.iter().map(|e| e.pc.clone()))
        .collect();
    let mut rows: BTreeMap<u16, Constant> = BTreeMap::new();
    for (_, e) in h.events() {
        let (pc, value, what, then) = match e {
            Ev::Branch {
                pc,
                test: Flags::Cmp { rhs, .. },
                ..
            } => match rhs.k8() {
                Some(k) => (*pc, k, "threshold", String::new()),
                None => continue,
            },
            Ev::Branch {
                pc,
                test: Flags::Res { mask: Some(m), .. },
                ..
            } => (*pc, *m, "mask", String::new()),
            Ev::Write { pc, loc, val, .. } => match val.k8() {
                Some(k) if matches!(loc, Loc::At(_) | Loc::Field { .. }) => {
                    (*pc, k, "stored", loc.hex())
                }
                _ => continue,
            },
            _ => continue,
        };
        // The instruction that holds the constant: the branch's compare is
        // before it; take the branch's own pc for the row.
        let p = format!("{pc:04X}");
        rows.entry(pc).or_insert_with(|| Constant {
            routine: h.rname(pc),
            pc: p.clone(),
            insn: h.r.insn(pc).text,
            value,
            what: what.into(),
            then,
            cited: cited.contains(&p),
        });
    }
    h.out.constants = rows.into_values().collect();
}
