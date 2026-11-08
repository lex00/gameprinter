//! Where the reader stopped: the routines the code map has that no path
//! read, and for each, the instruction in a routine it did read where it
//! didn't go on (a stop), with why. Stops are ranked by how many unread
//! routines are behind them, so each is a work item of its own
//! (`gp model static --unread`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Serialize;

use super::absint::{Ev, Reader};
use crate::codemap::{Mn, Mode, RomCode, Va, va_cpu, va_hex, va_tag};

/// Why the reader didn't go on at a stop.
pub const BANK: &str = "bank not known";
pub const POINTER: &str = "pointer not known";
pub const NOT_REACHED: &str = "site not reached";
pub const NO_CALLER: &str = "no caller";
pub const FOLLOWED: &str = "followed, not read";
pub const OTHER_BANK: &str = "another bank followed";
pub const CYCLE: &str = "only unread callers";

#[derive(Debug, Clone, Serialize)]
pub struct Stop {
    /// `reason@pc`: the same stop has the same id from run to run.
    pub id: String,
    pub reason: String,
    /// The instruction, and the routine it is in.
    pub pc: String,
    pub routine: String,
    /// The CPU address it calls or jumps to, when it names one.
    pub target: String,
    /// The RAM bytes that decide it: a pointer's two bytes.
    pub bytes: Vec<String>,
    /// Unread routines whose callers lead back to this stop.
    pub behind: usize,
    /// A few of them.
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Unread {
    pub schema: String,
    /// The PRG ROM's bytes, and those the code map has in a routine.
    pub prg_bytes: usize,
    pub code_bytes: usize,
    pub routines: usize,
    pub read: usize,
    pub unread: usize,
    /// Stops by reason, and the unread routines behind each reason.
    pub by_reason: BTreeMap<String, (usize, usize)>,
    pub stops: Vec<Stop>,
    /// The routines no path read, by name, for a search to aim at (not
    /// written to unread.json).
    #[serde(skip)]
    pub unread_routines: Vec<String>,
}

pub const SCHEMA: &str = "gp-static-unread/0.1.0";

/// The report on a pass's code map and reader.
pub fn report(code: &RomCode, r: &Reader) -> Unread {
    let rt = &code.routines;
    let n = rt.len();
    // What the reader ran: activations' starts and every event's pc.
    let mut ran: BTreeSet<Va> = r.acts.iter().map(|a| a.start).collect();
    let mut indirect: BTreeMap<Va, BTreeSet<String>> = BTreeMap::new();
    for a in &r.acts {
        for e in &a.events {
            let pc = match e {
                Ev::Read { pc, .. }
                | Ev::Write { pc, .. }
                | Ev::Step { pc, .. }
                | Ev::Branch { pc, .. }
                | Ev::Call { pc, .. } => *pc,
                Ev::Indirect { pc, lo, hi, .. } => {
                    let b = indirect.entry(*pc).or_default();
                    for l in lo.places().union(&hi.places()) {
                        if l.ram() {
                            b.insert(format!("${:02X}", l.addr));
                        }
                    }
                    *pc
                }
            };
            ran.insert(pc);
        }
    }
    let read: Vec<bool> = (0..n)
        .map(|i| {
            let a = rt.addr[i];
            ran.range(a..a + rt.size[i]).next().is_some()
        })
        .collect();
    let unresolved: BTreeSet<Va> = r.unresolved.iter().map(|&(pc, _)| pc).collect();
    // Static callers: (site, the routine the site is in) for each routine.
    let mut preds: Vec<Vec<(Va, usize)>> = vec![Vec::new(); n];
    let banked_by_cpu: BTreeMap<u16, Vec<usize>> = (0..n)
        .filter(|&i| va_tag(rt.addr[i]) != 0)
        .fold(BTreeMap::new(), |mut m, i| {
            m.entry(va_cpu(rt.addr[i])).or_default().push(i);
            m
        });
    let edge = |site: Va, to: Va, preds: &mut Vec<Vec<(Va, usize)>>| {
        let Some(c) = rt.at(site) else {
            return;
        };
        if let Some(j) = rt.at(to)
            && rt.addr[j] == to
            && j != c
        {
            preds[j].push((site, c));
        }
        // A call into a switched region names the routine in any bank.
        if va_tag(to) == 0
            && code.board.region(va_cpu(to)).is_some()
            && let Some(js) = banked_by_cpu.get(&va_cpu(to))
        {
            for &j in js {
                preds[j].push((site, c));
            }
        }
    };
    for ins in code.insns.values() {
        if matches!(ins.mn, Mn::Jsr) || (ins.mn == Mn::Jmp && ins.mode == Mode::Abs) {
            edge(ins.pc, ins.arg, &mut preds);
        }
    }
    for (&site, es) in &code.jumps {
        for &e in es {
            edge(site, e, &mut preds);
        }
    }
    let name = |i: usize| rt.names[i].clone();
    let mut stops: BTreeMap<String, Stop> = BTreeMap::new();
    let mut unread = 0;
    for i in (0..n).filter(|&i| !read[i]) {
        unread += 1;
        // Up the callers to the first that was read.
        let mut seen = BTreeSet::from([i]);
        let mut todo = VecDeque::from([i]);
        let mut found: BTreeSet<String> = BTreeSet::new();
        while let Some(j) = todo.pop_front() {
            // The top of a chain of unread callers.
            if preds[j].is_empty() {
                let id = format!("{NO_CALLER}@{}", va_hex(rt.addr[j]));
                stops.entry(id.clone()).or_insert_with(|| Stop {
                    id: id.clone(),
                    reason: NO_CALLER.into(),
                    pc: va_hex(rt.addr[j]),
                    routine: name(j),
                    target: String::new(),
                    bytes: Vec::new(),
                    behind: 0,
                    examples: Vec::new(),
                });
                found.insert(id);
            }
            for &(site, c) in &preds[j] {
                if !read[c] {
                    if seen.insert(c) {
                        todo.push_back(c);
                    }
                    continue;
                }
                let ins = code.insns.get(&site);
                let reason = if unresolved.contains(&site) {
                    BANK
                } else if ins.is_some_and(|x| x.mn == Mn::Jmp && x.mode == Mode::Ind) {
                    if ran.contains(&site) {
                        POINTER
                    } else {
                        NOT_REACHED
                    }
                } else if ran.contains(&site) {
                    // The call ran, into this address in another bank.
                    if va_tag(rt.addr[j]) != 0 {
                        OTHER_BANK
                    } else {
                        FOLLOWED
                    }
                } else {
                    NOT_REACHED
                };
                let id = format!("{reason}@{}", va_hex(site));
                stops.entry(id.clone()).or_insert_with(|| Stop {
                    id: id.clone(),
                    reason: reason.into(),
                    pc: va_hex(site),
                    routine: name(c),
                    target: ins
                        .filter(|x| x.mode != Mode::Ind)
                        .map(|x| format!("{:04X}", va_cpu(x.arg)))
                        .unwrap_or_default(),
                    bytes: indirect
                        .get(&site)
                        .map(|b| b.iter().cloned().collect())
                        .unwrap_or_default(),
                    behind: 0,
                    examples: Vec::new(),
                });
                found.insert(id);
            }
        }
        // Callers only among unread routines, round a cycle: the group is a
        // stop of its own, at its lowest routine.
        if found.is_empty()
            && let Some(&k) = seen.iter().min_by_key(|&&k| rt.addr[k])
        {
            let id = format!("{CYCLE}@{}", va_hex(rt.addr[k]));
            stops.entry(id.clone()).or_insert_with(|| Stop {
                id: id.clone(),
                reason: CYCLE.into(),
                pc: va_hex(rt.addr[k]),
                routine: name(k),
                target: String::new(),
                bytes: Vec::new(),
                behind: 0,
                examples: Vec::new(),
            });
            found.insert(id);
        }
        for id in found {
            if let Some(s) = stops.get_mut(&id) {
                s.behind += 1;
                if s.examples.len() < 4 {
                    s.examples.push(name(i));
                }
            }
        }
    }
    let mut stops: Vec<Stop> = stops.into_values().collect();
    stops.sort_by(|a, b| b.behind.cmp(&a.behind).then(a.id.cmp(&b.id)));
    let mut by_reason: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for s in &stops {
        let e = by_reason.entry(s.reason.clone()).or_default();
        e.0 += 1;
        e.1 += s.behind;
    }
    Unread {
        schema: SCHEMA.into(),
        prg_bytes: code.board.prg.len(),
        code_bytes: rt.size.iter().map(|&x| x as usize).sum(),
        routines: n,
        read: read.iter().filter(|&&x| x).count(),
        unread,
        by_reason,
        stops,
        unread_routines: (0..n)
            .filter(|&i| !read[i])
            .map(|i| rt.names[i].clone())
            .collect(),
    }
}
